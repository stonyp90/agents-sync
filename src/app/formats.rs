//! Key-level edits of JSON and TOML files that leave everything else in
//! the file alone.

use std::collections::BTreeMap;

use anyhow::{bail, Context, Result};
use serde_json::{Map, Value};
use toml_edit::{Array, DocumentMut, InlineTable, Item, Table};

use crate::domain::desired::KeyMode;

pub struct Edit<'a> {
    pub key: &'a str,
    pub value: &'a Value,
    pub mode: KeyMode,
}

pub struct Edited {
    pub content: String,
    /// Entry names now owned, per merged key.
    pub owned: BTreeMap<String, Vec<String>>,
    pub changed: bool,
}

type Owned = BTreeMap<String, Vec<String>>;

fn previously_owned<'a>(prev: &'a Owned, key: &str) -> &'a [String] {
    prev.get(key).map(Vec::as_slice).unwrap_or_default()
}

/// Creating an empty container the file never had is noise, not a change.
fn skip_empty(value: &Value, present: bool, prev: &[String]) -> bool {
    let empty = value.as_object().is_some_and(Map::is_empty);
    empty && !present && prev.is_empty()
}

pub fn edit_json(before: Option<&str>, edits: &[Edit], prev: &Owned) -> Result<Edited> {
    let original: Value = match before {
        Some(text) if !text.trim().is_empty() => {
            serde_json::from_str(text).context("not valid JSON")?
        }
        _ => Value::Object(Map::new()),
    };
    let mut doc = original.clone();
    let root = doc
        .as_object_mut()
        .context("top level is not a JSON object")?;
    let mut owned = Owned::new();

    for edit in edits {
        let prev_names = previously_owned(prev, edit.key);
        if skip_empty(edit.value, root.contains_key(edit.key), prev_names) {
            continue;
        }
        match edit.mode {
            KeyMode::Replace => {
                root.insert(edit.key.to_string(), edit.value.clone());
            }
            KeyMode::MergeOwned => {
                let desired = edit
                    .value
                    .as_object()
                    .context("merged value must be an object")?;
                let slot = root
                    .entry(edit.key.to_string())
                    .or_insert_with(|| Value::Object(Map::new()));
                let Some(entries) = slot.as_object_mut() else {
                    bail!("`{}` is not an object", edit.key);
                };
                for name in prev_names.iter().filter(|n| !desired.contains_key(*n)) {
                    entries.remove(name);
                }
                for (name, value) in desired {
                    entries.insert(name.clone(), value.clone());
                }
                owned.insert(edit.key.to_string(), desired.keys().cloned().collect());
            }
        }
    }

    let changed = doc != original;
    let content = if changed {
        serde_json::to_string_pretty(&doc)? + "\n"
    } else {
        before.unwrap_or_default().to_string()
    };
    Ok(Edited {
        content,
        owned,
        changed,
    })
}

pub fn edit_toml(before: Option<&str>, edits: &[Edit], prev: &Owned) -> Result<Edited> {
    let text = before.unwrap_or_default();
    let mut doc: DocumentMut = text.parse().context("not valid TOML")?;
    let mut owned = Owned::new();

    for edit in edits {
        if edit.mode != KeyMode::MergeOwned {
            bail!("only merged keys are supported in TOML files");
        }
        let prev_names = previously_owned(prev, edit.key);
        if skip_empty(edit.value, doc.contains_key(edit.key), prev_names) {
            continue;
        }
        let desired = edit
            .value
            .as_object()
            .context("merged value must be an object")?;
        if !doc.contains_key(edit.key) {
            let mut table = Table::new();
            table.set_implicit(true);
            doc.insert(edit.key, Item::Table(table));
        }
        let table = doc[edit.key]
            .as_table_mut()
            .with_context(|| format!("`{}` is not a table", edit.key))?;
        for name in prev_names.iter().filter(|n| !desired.contains_key(*n)) {
            table.remove(name);
        }
        for (name, value) in desired {
            let current = table.get(name).map(item_to_json);
            if current.as_ref() != Some(value) {
                table.insert(name, json_to_item(value)?);
            }
        }
        owned.insert(edit.key.to_string(), desired.keys().cloned().collect());
    }

    let content = doc.to_string();
    let changed = content != text;
    Ok(Edited {
        content,
        owned,
        changed,
    })
}

fn json_to_item(value: &Value) -> Result<Item> {
    match value {
        Value::Object(map) => {
            let mut table = Table::new();
            for (key, v) in map {
                table.insert(key, json_to_item(v)?);
            }
            Ok(Item::Table(table))
        }
        other => Ok(Item::Value(json_to_value(other)?)),
    }
}

fn json_to_value(value: &Value) -> Result<toml_edit::Value> {
    Ok(match value {
        Value::String(s) => s.as_str().into(),
        Value::Bool(b) => (*b).into(),
        Value::Number(n) => match n.as_i64() {
            Some(i) => i.into(),
            None => n.as_f64().context("number out of range")?.into(),
        },
        Value::Array(items) => {
            let mut array = Array::new();
            for item in items {
                array.push(json_to_value(item)?);
            }
            array.into()
        }
        Value::Object(map) => {
            let mut table = InlineTable::new();
            for (key, v) in map {
                table.insert(key, json_to_value(v)?);
            }
            table.into()
        }
        Value::Null => bail!("TOML has no null"),
    })
}

fn item_to_json(item: &Item) -> Value {
    match item {
        Item::None => Value::Null,
        Item::Value(v) => value_to_json(v),
        Item::Table(t) => Value::Object(
            t.iter()
                .map(|(k, v)| (k.to_string(), item_to_json(v)))
                .collect(),
        ),
        Item::ArrayOfTables(tables) => Value::Array(
            tables
                .iter()
                .map(|t| {
                    Value::Object(
                        t.iter()
                            .map(|(k, v)| (k.to_string(), item_to_json(v)))
                            .collect(),
                    )
                })
                .collect(),
        ),
    }
}

fn value_to_json(value: &toml_edit::Value) -> Value {
    use toml_edit::Value as V;
    match value {
        V::String(s) => Value::from(s.value().as_str()),
        V::Integer(i) => Value::from(*i.value()),
        V::Float(f) => Value::from(*f.value()),
        V::Boolean(b) => Value::from(*b.value()),
        V::Datetime(d) => Value::from(d.value().to_string()),
        V::Array(a) => Value::Array(a.iter().map(value_to_json).collect()),
        V::InlineTable(t) => Value::Object(
            t.iter()
                .map(|(k, v)| (k.to_string(), value_to_json(v)))
                .collect(),
        ),
    }
}

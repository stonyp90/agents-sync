//! agents-sync keeps one manifest as the source of truth for every coding
//! agent on the machine.
//!
//! Hexagonal layout:
//! - [`domain`]: the manifest model and the pure translation of it into the
//!   state each tool should be in. No I/O.
//! - [`ports`]: what the application needs from the outside world
//!   (a filesystem, a secret store).
//! - [`adapters`]: real disk, in-memory disk for tests, macOS Keychain.
//! - [`app`]: use cases (plan, sync, check, exec-mcp, watch) that wire the
//!   domain to the ports.

pub mod adapters;
pub mod app;
pub mod domain;
pub mod ports;

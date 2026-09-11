//! Git infrastructure for contained autoresearch runs.
//!
//! Commands execute directly through [`std::process::Command`]. No command
//! string passes through a shell.

mod repository;

pub use repository::{GitError, GitRepository};

//! Git infrastructure for contained autoresearch runs.
//!
//! Commands execute directly through [`std::process::Command`]. No command
//! string passes through a shell.

mod lock;
mod repository;

pub use lock::{RunLockGuard, RunLockOwner};
pub use repository::{GitError, GitRepository};

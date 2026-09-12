//! Git infrastructure for contained autoresearch runs.
//!
//! Commands execute directly through [`std::process::Command`]. No command
//! string passes through a shell.

mod error;
mod lock;
mod repository;
mod workspace;

pub use error::GitError;
pub use lock::{RunLockGuard, RunLockOwner};
pub use repository::GitRepository;
pub use workspace::LockedGitRepository;

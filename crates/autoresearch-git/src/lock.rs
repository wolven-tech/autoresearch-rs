//! Exclusive ownership lock for one experiment repository.

use crate::{GitError, GitRepository};
use autoresearch_core::{RepositoryInspector, RepositorySnapshot, RunId};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const LOCK_METADATA_LIMIT: u64 = 8 * 1024;

/// Stable metadata identifying current lock owner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunLockOwner {
    schema_version: u16,
    run_id: String,
    process_id: u32,
    acquired_unix_millis: u64,
}

impl RunLockOwner {
    /// Returns lock metadata schema version.
    #[must_use]
    pub const fn schema_version(&self) -> u16 {
        self.schema_version
    }

    /// Returns stable run identifier.
    #[must_use]
    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    /// Returns operating-system process identifier.
    #[must_use]
    pub const fn process_id(&self) -> u32 {
        self.process_id
    }

    /// Returns acquisition time in Unix milliseconds.
    #[must_use]
    pub const fn acquired_unix_millis(&self) -> u64 {
        self.acquired_unix_millis
    }
}

/// Exclusive repository ownership held until guard drops or process exits.
#[derive(Debug)]
pub struct RunLockGuard {
    file: File,
    path: PathBuf,
    owner: RunLockOwner,
}

impl RunLockGuard {
    /// Acquires exclusive experiment ownership for a validated repository.
    ///
    /// Lock file remains as evidence after release; kernel lock itself is
    /// released automatically when guard drops or process exits.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::LockHeld`] when another process owns repository,
    /// or a typed validation/filesystem error when lock cannot be made safely.
    pub fn acquire(snapshot: &RepositorySnapshot, run_id: &str) -> Result<Self, GitError> {
        let run_id = RunId::new(run_id).map_err(|_| GitError::InvalidRunId)?;
        let state_path = snapshot.root().join(".autoresearch");
        prepare_state_directory(&state_path)?;
        let lock_path = state_path.join("run.lock");
        reject_unsafe_lock_path(&lock_path)?;

        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|source| GitError::LockIo {
                operation: "open repository lock",
                path: lock_path.clone(),
                source,
            })?;
        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                return Err(GitError::LockHeld {
                    path: lock_path.clone(),
                    detail: read_lock_detail(&lock_path),
                });
            }
            Err(TryLockError::Error(source)) => {
                return Err(GitError::LockIo {
                    operation: "acquire repository lock",
                    path: lock_path,
                    source,
                });
            }
        }

        let current = GitRepository.inspect(snapshot.root(), snapshot.base_commit().as_str())?;
        if current != *snapshot {
            return Err(GitError::StaleSnapshot);
        }

        let owner = RunLockOwner {
            schema_version: 1,
            run_id: run_id.to_string(),
            process_id: std::process::id(),
            acquired_unix_millis: acquisition_time()?,
        };
        write_owner(&mut file, &lock_path, &owner)?;
        Ok(Self {
            file,
            path: lock_path,
            owner,
        })
    }

    /// Returns lock evidence path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns current owner metadata.
    #[must_use]
    pub const fn owner(&self) -> &RunLockOwner {
        &self.owner
    }
}

impl Drop for RunLockGuard {
    fn drop(&mut self) {
        // A child forked by another thread shares this open file until it execs,
        // so closing the handle alone can leave the flock held.
        let _ = self.file.unlock();
    }
}

fn prepare_state_directory(path: &Path) -> Result<(), GitError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(GitError::UnsafeRunState {
            path: path.to_path_buf(),
            reason: "run-state directory cannot be a symlink",
        }),
        Ok(metadata) if !metadata.is_dir() => Err(GitError::UnsafeRunState {
            path: path.to_path_buf(),
            reason: "run-state path must be a directory",
        }),
        Ok(_) => Ok(()),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => fs::create_dir(path)
            .map_err(|source| GitError::LockIo {
                operation: "create run-state directory",
                path: path.to_path_buf(),
                source,
            }),
        Err(source) => Err(GitError::LockIo {
            operation: "inspect run-state directory",
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn reject_unsafe_lock_path(path: &Path) -> Result<(), GitError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(GitError::UnsafeRunState {
            path: path.to_path_buf(),
            reason: "lock file cannot be a symlink",
        }),
        Ok(metadata) if !metadata.is_file() => Err(GitError::UnsafeRunState {
            path: path.to_path_buf(),
            reason: "lock path must be a regular file",
        }),
        Ok(_) => Ok(()),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(GitError::LockIo {
            operation: "inspect repository lock",
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn acquisition_time() -> Result<u64, GitError> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| GitError::ClockBeforeEpoch)?
        .as_millis();
    u64::try_from(millis).map_err(|_| GitError::ClockOverflow)
}

fn write_owner(file: &mut File, path: &Path, owner: &RunLockOwner) -> Result<(), GitError> {
    file.set_len(0)
        .map_err(|source| lock_io("truncate repository lock", path, source))?;
    file.seek(SeekFrom::Start(0))
        .map_err(|source| lock_io("seek repository lock", path, source))?;
    serde_json::to_writer(&mut *file, owner)?;
    file.write_all(b"\n")
        .map_err(|source| lock_io("write repository lock", path, source))?;
    file.sync_data()
        .map_err(|source| lock_io("sync repository lock", path, source))
}

fn read_lock_detail(path: &Path) -> String {
    let Ok(file) = File::open(path) else {
        return "owner metadata unavailable".into();
    };
    let mut detail = String::new();
    if file
        .take(LOCK_METADATA_LIMIT)
        .read_to_string(&mut detail)
        .is_err()
    {
        return "owner metadata unreadable".into();
    }
    let detail = detail.trim();
    if detail.is_empty() {
        "owner metadata empty".into()
    } else {
        detail.to_owned()
    }
}

fn lock_io(operation: &'static str, path: &Path, source: std::io::Error) -> GitError {
    GitError::LockIo {
        operation,
        path: path.to_path_buf(),
        source,
    }
}

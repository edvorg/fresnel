//! The lock that lets only one process of the app run (plan section 11.6).
//!
//! Two processes would write the same settings files and bind the same local ports. The
//! lock is a lock of the system on a file of the data folder. The system frees it when the
//! process ends, also after a crash, so no old lock file can stop a start.

use std::fmt::{self, Display};
use std::fs::{File, OpenOptions, TryLockError};
use std::io::{self, Read as _, Seek as _, Write as _};
use std::path::Path;

const FILE: &str = "fresnel.lock";

/// Why the process did not get the lock.
#[derive(Debug)]
pub enum LockError {
    /// Some other process of the app runs. It wrote its process id into the file.
    Held {
        /// The process id of the other process, if the file has one.
        process: Option<u32>,
    },
    /// The lock file could not be used.
    File(io::Error),
}

impl Display for LockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Held { process: Some(process) } => write!(f, "fresnel runs already (process {process})"),
            Self::Held { process: None } => f.write_str("fresnel runs already"),
            Self::File(error) => write!(f, "cannot use the lock file: {error}"),
        }
    }
}

/// The lock of this process. The lock is free again when this value drops.
#[derive(Debug)]
pub struct ProcessLock {
    _file: File,
}

impl ProcessLock {
    /// Takes the lock of a data folder. The call does not wait for some other process.
    pub fn take(data_folder: &Path) -> Result<Self, LockError> {
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(data_folder.join(FILE))
            .map_err(LockError::File)?;
        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => return Err(LockError::Held { process: read_process(&mut file) }),
            Err(TryLockError::Error(error)) => return Err(LockError::File(error)),
        }
        write_process(&mut file).map_err(LockError::File)?;
        Ok(Self { _file: file })
    }
}

fn read_process(file: &mut File) -> Option<u32> {
    let mut text = String::new();
    file.read_to_string(&mut text).ok()?;
    text.trim().parse().ok()
}

fn write_process(file: &mut File) -> io::Result<()> {
    file.set_len(0)?;
    file.rewind()?;
    writeln!(file, "{}", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::Scratch;

    #[test]
    fn a_second_lock_of_the_same_folder_fails_with_the_process_of_the_first() {
        let folder = Scratch::new("lock");
        let first = ProcessLock::take(folder.path()).expect("the first lock");
        let second = ProcessLock::take(folder.path()).expect_err("the folder has its lock");
        assert!(matches!(second, LockError::Held { process: Some(process) } if process == std::process::id()));
        assert_eq!(second.to_string(), format!("fresnel runs already (process {})", std::process::id()));

        // The end of the first process frees the lock. The old file does not stop a start.
        drop(first);
        assert!(folder.path().join(FILE).exists());
        ProcessLock::take(folder.path()).expect("the lock is free again");
    }

    #[test]
    fn a_folder_that_is_absent_and_a_file_with_no_process_id_give_their_error() {
        let folder = Scratch::new("lock-errors");
        let absent = ProcessLock::take(&folder.path().join("absent")).expect_err("no folder");
        assert!(absent.to_string().starts_with("cannot use the lock file: "), "{absent}");

        let held = File::create(folder.path().join(FILE)).expect("the lock file");
        held.try_lock().expect("a lock by some other owner");
        let error = ProcessLock::take(folder.path()).expect_err("the file has its lock");
        assert_eq!(error.to_string(), "fresnel runs already");
    }
}

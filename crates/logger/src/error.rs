use std::path::PathBuf;

/// Why the logger could not start.
#[derive(Debug)]
pub enum Error {
    /// A logger already ran in this process. There is one per process, and it
    /// cannot be restarted after a shutdown.
    AlreadyInitialized,
    /// The I/O thread could not be spawned.
    FailedToSpawnThread(std::io::Error),
    /// The write buffer is larger than the maximum file size, so a single
    /// flush could overrun a file.
    BufferBiggerThanFile {
        /// Buffer size asked for, in bytes.
        buffer_size: u64,
        /// Maximum file size, in bytes.
        file_size: u64,
    },
    /// The log directory or the first log file could not be created.
    OpenLogFile {
        /// The log directory.
        path: PathBuf,
        /// The underlying I/O error.
        source: std::io::Error,
    },
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::AlreadyInitialized => f.write_str("the logger is already initialised"),
            Error::FailedToSpawnThread(_) => f.write_str("failed to spawn the logger thread"),
            Error::OpenLogFile { path, .. } => {
                write!(f, "failed to open a log file in {}", path.display())
            }
            Error::BufferBiggerThanFile {
                buffer_size,
                file_size,
            } => write!(
                f,
                "write buffer ({buffer_size} bytes) is larger than the maximum file size ({file_size} bytes)"
            ),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::FailedToSpawnThread(source) | Error::OpenLogFile { source, .. } => Some(source),
            Error::AlreadyInitialized | Error::BufferBiggerThanFile { .. } => None,
        }
    }
}

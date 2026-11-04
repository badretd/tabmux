//! Error types and stable process exit codes.
//!
//! The exit codes are part of the CLI contract (see `docs/features.md`):
//!
//! | Code | Meaning                                      |
//! |------|----------------------------------------------|
//! | 0    | Success                                      |
//! | 1    | User/data error                              |
//! | 2    | CLI usage error                              |
//! | 3    | I/O error                                    |
//! | 4    | Unsupported format or feature                |
//! | 5    | Lossy conversion blocked by `--strict`       |
//! | 6    | Partial success                              |
//! | 130  | Interrupted                                  |

use std::process::ExitCode as ProcessExitCode;

use thiserror::Error;

/// Stable exit codes shared by every command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ExitCode {
    Success = 0,
    UserData = 1,
    Usage = 2,
    Io = 3,
    Unsupported = 4,
    LossyBlocked = 5,
    Partial = 6,
    Interrupted = 130,
}

impl ExitCode {
    /// Numeric value suitable for `std::process::exit`.
    pub const fn code(self) -> u8 {
        self as u8
    }

    /// Convert to the standard library exit code type.
    pub fn process(self) -> ProcessExitCode {
        ProcessExitCode::from(self.code())
    }
}

/// Application error with a stable mapping to an [`ExitCode`].
#[derive(Debug, Error)]
pub enum Error {
    /// Invalid command-line combination or manifest value.
    #[error("{message}")]
    Usage { message: String },

    /// Invalid, corrupt, or unreadable user data.
    #[error("{message}")]
    Data { message: String },

    /// Filesystem or stream failure.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Format or feature that this build does not support.
    #[error("unsupported format or feature: {0}")]
    Unsupported(String),

    /// Conversion would lose information and `--strict` is active.
    #[error("conversion blocked by strict mode: {0}")]
    LossyBlocked(String),

    /// Some inputs or outputs succeeded while others failed.
    #[error("partial success: {0}")]
    Partial(String),

    /// Operation was interrupted by the user.
    #[error("interrupted")]
    Interrupted,
}

impl Error {
    /// Build a usage error.
    pub fn usage(message: impl Into<String>) -> Self {
        Self::Usage {
            message: message.into(),
        }
    }

    /// Build a user/data error.
    pub fn data(message: impl Into<String>) -> Self {
        Self::Data {
            message: message.into(),
        }
    }

    /// Build an unsupported-feature error.
    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::Unsupported(message.into())
    }

    /// The process exit code associated with this error.
    pub const fn exit_code(&self) -> ExitCode {
        match self {
            Self::Usage { .. } => ExitCode::Usage,
            Self::Data { .. } => ExitCode::UserData,
            Self::Io(_) => ExitCode::Io,
            Self::Unsupported(_) => ExitCode::Unsupported,
            Self::LossyBlocked(_) => ExitCode::LossyBlocked,
            Self::Partial(_) => ExitCode::Partial,
            Self::Interrupted => ExitCode::Interrupted,
        }
    }
}

/// Convenient result alias for fallible library operations.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_match_the_documented_contract() {
        assert_eq!(ExitCode::Success.code(), 0);
        assert_eq!(ExitCode::UserData.code(), 1);
        assert_eq!(ExitCode::Usage.code(), 2);
        assert_eq!(ExitCode::Io.code(), 3);
        assert_eq!(ExitCode::Unsupported.code(), 4);
        assert_eq!(ExitCode::LossyBlocked.code(), 5);
        assert_eq!(ExitCode::Partial.code(), 6);
        assert_eq!(ExitCode::Interrupted.code(), 130);
    }

    #[test]
    fn errors_map_to_their_exit_codes() {
        assert_eq!(Error::usage("bad flag").exit_code(), ExitCode::Usage);
        assert_eq!(Error::data("bad row").exit_code(), ExitCode::UserData);
        assert_eq!(Error::unsupported("pdf").exit_code(), ExitCode::Unsupported);
        assert_eq!(
            Error::LossyBlocked("chart".into()).exit_code(),
            ExitCode::LossyBlocked
        );
        assert_eq!(
            Error::Partial("one input".into()).exit_code(),
            ExitCode::Partial
        );
        assert_eq!(Error::Interrupted.exit_code(), ExitCode::Interrupted);
    }
}

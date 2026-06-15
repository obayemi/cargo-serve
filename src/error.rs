use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no binary target found in package")]
    NoBinaryTarget,

    #[error("multiple binary targets found: {targets:?} — use --bin to select one")]
    MultipleBinaryTargets { targets: Vec<String> },

    #[error("binary target {name:?} not found in package")]
    BinaryTargetNotFound { name: String },

    #[error("cargo check failed")]
    CheckFailed,

    #[error("cargo build failed")]
    BuildFailed,

    #[error("could not find built binary in cargo output")]
    BinaryNotFoundInOutput,

    #[error("failed to stage binary to {path}")]
    StagingFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    // The wrapped source is intentionally left out of these Display strings:
    // `chain` appends it from `source()`, so embedding it here too would print
    // the underlying cause twice.
    #[error("failed to spawn server")]
    SpawnFailed(#[source] std::io::Error),

    #[error("failed to stop server")]
    StopFailed(#[source] std::io::Error),

    #[error("file watcher error")]
    Watcher(#[from] notify::Error),

    #[error("cargo metadata error")]
    Metadata(#[from] cargo_metadata::Error),

    #[error("io error")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Render an error together with its full `source` chain as a single line,
/// e.g. `failed to stage binary to /x: No such file or directory (os error 2)`.
///
/// `Display` on a `thiserror` enum only prints the top-level message, so the
/// underlying cause (an io error, a cargo error) is otherwise lost when logged.
pub fn chain(err: &dyn std::error::Error) -> String {
    let mut out = err.to_string();
    let mut source = err.source();
    while let Some(cause) = source {
        out.push_str(": ");
        out.push_str(&cause.to_string());
        source = cause.source();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chain_appends_source_cause() {
        let err = Error::StagingFailed {
            path: PathBuf::from("/x/bin"),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "no such file"),
        };
        assert_eq!(
            chain(&err),
            "failed to stage binary to /x/bin: no such file"
        );
    }

    #[test]
    fn chain_without_source_is_just_the_message() {
        assert_eq!(
            chain(&Error::NoBinaryTarget),
            "no binary target found in package"
        );
    }

    #[test]
    fn chain_does_not_duplicate_a_wrapped_source() {
        // Variants that carry a `#[source]` must not also embed it in their own
        // Display, or `chain` renders the cause twice.
        let err = Error::SpawnFailed(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "denied",
        ));
        assert_eq!(chain(&err), "failed to spawn server: denied");
    }
}

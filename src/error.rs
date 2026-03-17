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

    #[error("failed to spawn server: {0}")]
    SpawnFailed(#[source] std::io::Error),

    #[error("failed to stop server: {0}")]
    StopFailed(#[source] std::io::Error),

    #[error("file watcher error: {0}")]
    Watcher(#[from] notify::Error),

    #[error("cargo metadata error: {0}")]
    Metadata(#[from] cargo_metadata::Error),

    #[error("{0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

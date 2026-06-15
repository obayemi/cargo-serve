use std::path::PathBuf;

use cargo_metadata::{MetadataCommand, TargetKind};

use crate::error::{Error, Result};

#[derive(Debug)]
pub struct ProjectInfo {
    pub package_name: String,
    pub bin_name: String,
    pub target_dir: PathBuf,
    pub workspace_root: PathBuf,
}

impl ProjectInfo {
    pub fn discover(bin: Option<&str>) -> Result<Self> {
        let metadata = MetadataCommand::new().exec()?;

        let root_package = metadata.root_package().ok_or(Error::NoBinaryTarget)?;

        let bin_target_names: Vec<&str> = root_package
            .targets
            .iter()
            .filter(|t| t.kind.contains(&TargetKind::Bin))
            .map(|t| t.name.as_str())
            .collect();

        let bin_name =
            resolve_bin_name(bin, &bin_target_names, root_package.default_run.as_deref())?;

        Ok(Self {
            package_name: root_package.name.to_string(),
            bin_name,
            target_dir: metadata.target_directory.into_std_path_buf(),
            workspace_root: metadata.workspace_root.into_std_path_buf(),
        })
    }

    /// Directory where staged binaries are placed.
    pub fn staging_dir(&self) -> PathBuf {
        self.target_dir.join(".cargo-serve")
    }

    /// Path to the staged binary.
    pub fn staged_binary(&self) -> PathBuf {
        self.staging_dir().join(&self.bin_name)
    }
}

/// Select which binary target to run.
///
/// Precedence: an explicit `--bin` selection, then the package's `default-run`,
/// then the sole binary when there is only one. With multiple binaries and no
/// `--bin`/`default-run`, the caller must disambiguate.
fn resolve_bin_name(
    requested: Option<&str>,
    bin_targets: &[&str],
    default_run: Option<&str>,
) -> Result<String> {
    if let Some(name) = requested {
        return if bin_targets.contains(&name) {
            Ok(name.to_string())
        } else {
            Err(Error::BinaryTargetNotFound {
                name: name.to_string(),
            })
        };
    }

    if let Some(name) = default_run {
        return if bin_targets.contains(&name) {
            Ok(name.to_string())
        } else {
            Err(Error::BinaryTargetNotFound {
                name: name.to_string(),
            })
        };
    }

    match bin_targets {
        [] => Err(Error::NoBinaryTarget),
        [only] => Ok(only.to_string()),
        _ => Err(Error::MultipleBinaryTargets {
            targets: bin_targets.iter().map(|t| t.to_string()).collect(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discover_current_project() {
        let info = ProjectInfo::discover(None).expect("should discover cargo-serve itself");
        assert_eq!(info.package_name, "cargo-serve");
        assert_eq!(info.bin_name, "cargo-serve");
    }

    #[test]
    fn discover_nonexistent_bin() {
        let err = ProjectInfo::discover(Some("nonexistent")).unwrap_err();
        assert!(matches!(err, Error::BinaryTargetNotFound { .. }));
    }

    #[test]
    fn resolve_single_target_without_flag() {
        let name = resolve_bin_name(None, &["only"], None).unwrap();
        assert_eq!(name, "only");
    }

    #[test]
    fn resolve_explicit_flag_wins_over_default_run() {
        let name = resolve_bin_name(Some("a"), &["a", "b"], Some("b")).unwrap();
        assert_eq!(name, "a");
    }

    #[test]
    fn resolve_explicit_flag_unknown_errors() {
        let err = resolve_bin_name(Some("missing"), &["a", "b"], None).unwrap_err();
        assert!(matches!(err, Error::BinaryTargetNotFound { .. }));
    }

    #[test]
    fn resolve_multiple_targets_uses_default_run() {
        let name = resolve_bin_name(None, &["a", "b"], Some("b")).unwrap();
        assert_eq!(name, "b");
    }

    #[test]
    fn resolve_multiple_targets_without_default_run_errors() {
        let err = resolve_bin_name(None, &["a", "b"], None).unwrap_err();
        assert!(matches!(err, Error::MultipleBinaryTargets { .. }));
    }

    #[test]
    fn resolve_default_run_pointing_at_unknown_target_errors() {
        let err = resolve_bin_name(None, &["a", "b"], Some("ghost")).unwrap_err();
        assert!(matches!(err, Error::BinaryTargetNotFound { .. }));
    }

    #[test]
    fn resolve_no_targets_errors() {
        let err = resolve_bin_name(None, &[], None).unwrap_err();
        assert!(matches!(err, Error::NoBinaryTarget));
    }
}

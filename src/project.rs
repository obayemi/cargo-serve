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

        let bin_targets: Vec<_> = root_package
            .targets
            .iter()
            .filter(|t| t.kind.contains(&TargetKind::Bin))
            .collect();

        let bin_name = match bin {
            Some(name) => {
                if bin_targets.iter().any(|t| t.name.as_str() == name) {
                    name.to_string()
                } else {
                    return Err(Error::BinaryTargetNotFound {
                        name: name.to_string(),
                    });
                }
            }
            None => match bin_targets.len() {
                0 => return Err(Error::NoBinaryTarget),
                1 => bin_targets[0].name.to_string(),
                _ => {
                    return Err(Error::MultipleBinaryTargets {
                        targets: bin_targets.iter().map(|t| t.name.to_string()).collect(),
                    });
                }
            },
        };

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
}

use std::path::{Path, PathBuf};

use cargo_metadata::{Metadata, MetadataCommand, Package, TargetKind};

use crate::cli::CargoArgs;
use crate::error::{Error, Result};

/// Which kind of executable target is being served.
///
/// `cargo run` can launch a binary or an example; both produce an executable,
/// but examples land in an `examples/` subdirectory of the profile directory.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TargetKindSelector {
    #[default]
    Bin,
    Example,
}

impl TargetKindSelector {
    /// The cargo flag that selects this kind of target.
    pub fn cargo_flag(self) -> &'static str {
        match self {
            Self::Bin => "--bin",
            Self::Example => "--example",
        }
    }

    /// The `kind` string cargo reports in `compiler-artifact` messages.
    pub fn artifact_kind(self) -> &'static str {
        match self {
            Self::Bin => "bin",
            Self::Example => "example",
        }
    }

    /// Subdirectory of the profile directory holding this kind of artifact.
    pub fn artifact_subdir(self) -> Option<&'static str> {
        match self {
            Self::Bin => None,
            Self::Example => Some("examples"),
        }
    }
}

#[derive(Debug)]
pub struct ProjectInfo {
    pub package_name: String,
    pub target_name: String,
    pub target_kind: TargetKindSelector,
    pub target_dir: PathBuf,
    pub workspace_root: PathBuf,
}

impl ProjectInfo {
    pub fn discover(args: &CargoArgs) -> Result<Self> {
        let metadata = load_metadata(args)?;
        let package = select_package(&metadata, args.package.as_deref())?;

        let (target_name, target_kind) = resolve_target(
            args.bin.as_deref(),
            args.example.as_deref(),
            &target_names(package, TargetKind::Bin),
            &target_names(package, TargetKind::Example),
            package.default_run.as_deref(),
            package.name.as_str(),
        )?;

        Ok(Self {
            package_name: package.name.to_string(),
            target_name,
            target_kind,
            // `--target-dir` isn't reflected in `cargo metadata`'s output, so the
            // override has to be applied here for artifact paths to line up.
            target_dir: match &args.target_dir {
                Some(dir) => resolve_target_dir(dir)?,
                None => metadata.target_directory.into_std_path_buf(),
            },
            workspace_root: metadata.workspace_root.into_std_path_buf(),
        })
    }

    /// Directory where staged binaries are placed.
    pub fn staging_dir(&self) -> PathBuf {
        self.target_dir.join(".cargo-serve")
    }

    /// Path to the staged binary.
    pub fn staged_binary(&self) -> PathBuf {
        self.staging_dir().join(&self.target_name)
    }
}

/// Run `cargo metadata` under the same manifest and network rules as the build,
/// so both resolve the same workspace.
fn load_metadata(args: &CargoArgs) -> Result<Metadata> {
    let mut cmd = MetadataCommand::new();
    cmd.other_options(
        args.metadata_flags()
            .into_iter()
            .map(|f| f.to_string_lossy().into_owned())
            .collect::<Vec<_>>(),
    );
    Ok(cmd.exec()?)
}

/// Resolve a `--target-dir` the way cargo does — relative to the invocation
/// directory — and match the shape of `cargo metadata`'s `target_directory`.
///
/// The absolute, symlink-resolved form matters beyond cosmetics: the watcher
/// recognises the build output directory by comparing it against the workspace
/// root, and a relative path inside the workspace would slip past that check and
/// make every build trigger the next one.
fn resolve_target_dir(dir: &Path) -> Result<PathBuf> {
    let absolute = std::path::absolute(dir)?;
    // `canonicalize` needs the directory to exist; on the first run it doesn't.
    Ok(absolute.canonicalize().unwrap_or(absolute))
}

fn target_names(package: &Package, kind: TargetKind) -> Vec<&str> {
    package
        .targets
        .iter()
        .filter(|t| t.kind.contains(&kind))
        .map(|t| t.name.as_str())
        .collect()
}

/// Pick the workspace package to serve.
///
/// Mirrors `cargo run`: an explicit `-p` wins, otherwise the package in the
/// current directory. In a virtual workspace with several members there is no
/// current package, so `-p` is required.
fn select_package<'a>(metadata: &'a Metadata, spec: Option<&str>) -> Result<&'a Package> {
    let members = metadata.workspace_packages();

    if let Some(spec) = spec {
        return members
            .iter()
            .copied()
            .find(|p| package_matches_spec(p, spec))
            .ok_or_else(|| Error::PackageNotFound {
                spec: spec.to_string(),
                available: members.iter().map(|p| p.name.to_string()).collect(),
            });
    }

    if let Some(root) = metadata.root_package() {
        return Ok(root);
    }

    match members.as_slice() {
        [only] => Ok(only),
        _ => Err(Error::NoCurrentPackage {
            available: members.iter().map(|p| p.name.to_string()).collect(),
        }),
    }
}

/// Match a package against a cargo package spec, supporting the two spellings
/// that appear on a command line: `name` and `name@version`.
fn package_matches_spec(package: &Package, spec: &str) -> bool {
    match spec.split_once('@') {
        Some((name, version)) => {
            package.name.as_str() == name && package.version.to_string() == version
        }
        None => package.name.as_str() == spec,
    }
}

/// Select which executable target to run.
///
/// Precedence: an explicit `--example`, then `--bin`, then the package's
/// `default-run`, then the sole binary when there is only one. With multiple
/// binaries and no `--bin`/`default-run`, the caller must disambiguate.
fn resolve_target(
    requested_bin: Option<&str>,
    requested_example: Option<&str>,
    bin_targets: &[&str],
    example_targets: &[&str],
    default_run: Option<&str>,
    package: &str,
) -> Result<(String, TargetKindSelector)> {
    let require = |name: &str, kind: TargetKindSelector, available: &[&str]| {
        if available.contains(&name) {
            Ok((name.to_string(), kind))
        } else {
            Err(Error::TargetNotFound {
                kind: kind.artifact_kind(),
                name: name.to_string(),
                package: package.to_string(),
            })
        }
    };

    if let Some(name) = requested_example {
        return require(name, TargetKindSelector::Example, example_targets);
    }
    if let Some(name) = requested_bin {
        return require(name, TargetKindSelector::Bin, bin_targets);
    }
    if let Some(name) = default_run {
        return require(name, TargetKindSelector::Bin, bin_targets);
    }

    match bin_targets {
        [] => Err(Error::NoBinaryTarget {
            package: package.to_string(),
        }),
        [only] => Ok((only.to_string(), TargetKindSelector::Bin)),
        _ => Err(Error::MultipleBinaryTargets {
            targets: bin_targets.iter().map(|t| t.to_string()).collect(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolve(
        bin: Option<&str>,
        example: Option<&str>,
        bins: &[&str],
        examples: &[&str],
        default_run: Option<&str>,
    ) -> Result<(String, TargetKindSelector)> {
        resolve_target(bin, example, bins, examples, default_run, "pkg")
    }

    #[test]
    fn discover_current_project() {
        let info = ProjectInfo::discover(&CargoArgs::default())
            .expect("should discover cargo-serve itself");
        assert_eq!(info.package_name, "cargo-serve");
        assert_eq!(info.target_name, "cargo-serve");
        assert_eq!(info.target_kind, TargetKindSelector::Bin);
    }

    #[test]
    fn discover_nonexistent_bin() {
        let args = CargoArgs {
            bin: Some("nonexistent".into()),
            ..Default::default()
        };
        let err = ProjectInfo::discover(&args).unwrap_err();
        assert!(matches!(err, Error::TargetNotFound { .. }));
    }

    #[test]
    fn discover_nonexistent_package() {
        let args = CargoArgs {
            package: Some("nonexistent".into()),
            ..Default::default()
        };
        let err = ProjectInfo::discover(&args).unwrap_err();
        assert!(matches!(err, Error::PackageNotFound { .. }));
    }

    #[test]
    fn discover_honours_target_dir_override() {
        let args = CargoArgs {
            target_dir: Some(PathBuf::from("/tmp/elsewhere")),
            ..Default::default()
        };
        let info = ProjectInfo::discover(&args).unwrap();
        assert_eq!(info.target_dir, PathBuf::from("/tmp/elsewhere"));
        assert_eq!(
            info.staged_binary(),
            PathBuf::from("/tmp/elsewhere/.cargo-serve/cargo-serve")
        );
    }

    #[test]
    fn relative_target_dir_becomes_absolute() {
        // A relative override left as-is can't be compared against the workspace
        // root, so the watcher would keep rebuilding its own output.
        let resolved = resolve_target_dir(Path::new("out")).unwrap();
        assert!(
            resolved.is_absolute(),
            "{} must be absolute",
            resolved.display()
        );
        assert!(resolved.ends_with("out"));
    }

    #[test]
    fn absolute_target_dir_is_preserved_even_when_missing() {
        assert_eq!(
            resolve_target_dir(Path::new("/nonexistent/cargo-serve-out")).unwrap(),
            PathBuf::from("/nonexistent/cargo-serve-out")
        );
    }

    #[test]
    fn resolve_single_target_without_flag() {
        assert_eq!(
            resolve(None, None, &["only"], &[], None).unwrap(),
            ("only".to_string(), TargetKindSelector::Bin)
        );
    }

    #[test]
    fn resolve_explicit_flag_wins_over_default_run() {
        let (name, _) = resolve(Some("a"), None, &["a", "b"], &[], Some("b")).unwrap();
        assert_eq!(name, "a");
    }

    #[test]
    fn resolve_explicit_flag_unknown_errors() {
        let err = resolve(Some("missing"), None, &["a", "b"], &[], None).unwrap_err();
        assert!(matches!(err, Error::TargetNotFound { kind: "bin", .. }));
    }

    #[test]
    fn resolve_multiple_targets_uses_default_run() {
        let (name, _) = resolve(None, None, &["a", "b"], &[], Some("b")).unwrap();
        assert_eq!(name, "b");
    }

    #[test]
    fn resolve_multiple_targets_without_default_run_errors() {
        let err = resolve(None, None, &["a", "b"], &[], None).unwrap_err();
        assert!(matches!(err, Error::MultipleBinaryTargets { .. }));
    }

    #[test]
    fn resolve_default_run_pointing_at_unknown_target_errors() {
        let err = resolve(None, None, &["a", "b"], &[], Some("ghost")).unwrap_err();
        assert!(matches!(err, Error::TargetNotFound { .. }));
    }

    #[test]
    fn resolve_no_targets_errors() {
        let err = resolve(None, None, &[], &[], None).unwrap_err();
        assert!(matches!(err, Error::NoBinaryTarget { .. }));
    }

    #[test]
    fn resolve_example_selects_the_example_kind() {
        assert_eq!(
            resolve(None, Some("demo"), &["app"], &["demo"], Some("app")).unwrap(),
            ("demo".to_string(), TargetKindSelector::Example)
        );
    }

    #[test]
    fn resolve_example_is_not_satisfied_by_a_binary_of_the_same_name() {
        let err = resolve(None, Some("app"), &["app"], &[], None).unwrap_err();
        assert!(matches!(
            err,
            Error::TargetNotFound {
                kind: "example",
                ..
            }
        ));
    }

    #[test]
    fn example_artifacts_live_in_an_examples_subdirectory() {
        assert_eq!(TargetKindSelector::Bin.artifact_subdir(), None);
        assert_eq!(
            TargetKindSelector::Example.artifact_subdir(),
            Some("examples")
        );
    }

    #[test]
    fn spec_matches_bare_name_and_name_at_version() {
        let package = ProjectInfo::discover(&CargoArgs::default()).unwrap();
        // Reuse the real metadata to get a Package without hand-rolling one.
        let metadata = load_metadata(&CargoArgs::default()).unwrap();
        let pkg = select_package(&metadata, Some(&package.package_name)).unwrap();

        assert!(package_matches_spec(pkg, "cargo-serve"));
        assert!(package_matches_spec(
            pkg,
            &format!("cargo-serve@{}", pkg.version)
        ));
        assert!(!package_matches_spec(pkg, "cargo-serve@0.0.0"));
        assert!(!package_matches_spec(pkg, "other"));
    }
}

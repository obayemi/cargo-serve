//! Target selection against a real `cargo metadata` run over a virtual
//! workspace — the case `cargo run -p <pkg>` handles and `cargo serve` has to
//! match.

use std::fs;
use std::path::{Path, PathBuf};

use cargo_serve::cli::CargoArgs;
use cargo_serve::error::Error;
use cargo_serve::project::{ProjectInfo, TargetKindSelector};

/// A virtual workspace with two members:
///
/// - `alpha`: two binaries (`alpha`, `alpha-worker`), `default-run = "alpha"`,
///   and an example named `demo`
/// - `beta`: a single binary
fn workspace() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    write(
        &root.join("Cargo.toml"),
        r#"[workspace]
members = ["alpha", "beta"]
resolver = "2"
"#,
    );

    write(
        &root.join("alpha/Cargo.toml"),
        r#"[package]
name = "alpha"
version = "0.1.0"
edition = "2021"
default-run = "alpha"

[[bin]]
name = "alpha"
path = "src/main.rs"

[[bin]]
name = "alpha-worker"
path = "src/worker.rs"

[[example]]
name = "demo"
path = "examples/demo.rs"
"#,
    );
    write(&root.join("alpha/src/main.rs"), "fn main() {}\n");
    write(&root.join("alpha/src/worker.rs"), "fn main() {}\n");
    write(&root.join("alpha/examples/demo.rs"), "fn main() {}\n");

    write(
        &root.join("beta/Cargo.toml"),
        r#"[package]
name = "beta"
version = "2.3.4"
edition = "2021"
"#,
    );
    write(&root.join("beta/src/main.rs"), "fn main() {}\n");

    dir
}

fn write(path: &Path, contents: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

fn args(root: &Path) -> CargoArgs {
    CargoArgs {
        manifest_path: Some(root.join("Cargo.toml")),
        ..Default::default()
    }
}

#[test]
fn package_flag_selects_a_workspace_member_and_its_default_run() {
    let ws = workspace();
    let info = ProjectInfo::discover(&CargoArgs {
        package: Some("alpha".into()),
        ..args(ws.path())
    })
    .unwrap();

    assert_eq!(info.package_name, "alpha");
    assert_eq!(info.target_name, "alpha");
    assert_eq!(info.target_kind, TargetKindSelector::Bin);
    assert_eq!(info.workspace_root, ws.path().canonicalize().unwrap());
    assert_eq!(
        info.target_dir,
        ws.path().canonicalize().unwrap().join("target")
    );
}

#[test]
fn package_flag_combines_with_bin_selection() {
    let ws = workspace();
    let info = ProjectInfo::discover(&CargoArgs {
        package: Some("alpha".into()),
        bin: Some("alpha-worker".into()),
        ..args(ws.path())
    })
    .unwrap();

    assert_eq!(info.target_name, "alpha-worker");
    assert_eq!(info.target_kind, TargetKindSelector::Bin);
}

#[test]
fn package_flag_accepts_name_at_version() {
    let ws = workspace();
    let info = ProjectInfo::discover(&CargoArgs {
        package: Some("beta@2.3.4".into()),
        ..args(ws.path())
    })
    .unwrap();

    assert_eq!(info.package_name, "beta");
    assert_eq!(info.target_name, "beta");
}

#[test]
fn example_selection_resolves_to_the_examples_artifact_path() {
    let ws = workspace();
    let info = ProjectInfo::discover(&CargoArgs {
        package: Some("alpha".into()),
        example: Some("demo".into()),
        ..args(ws.path())
    })
    .unwrap();

    assert_eq!(info.target_name, "demo");
    assert_eq!(info.target_kind, TargetKindSelector::Example);
    assert_eq!(
        info.staged_binary(),
        ws.path()
            .canonicalize()
            .unwrap()
            .join("target/.cargo-serve/demo")
    );
}

#[test]
fn virtual_workspace_root_without_package_flag_asks_for_one() {
    let ws = workspace();
    let err = ProjectInfo::discover(&args(ws.path())).unwrap_err();

    let Error::NoCurrentPackage { available } = err else {
        panic!("expected NoCurrentPackage, got {err:?}");
    };
    let mut available = available;
    available.sort();
    assert_eq!(available, vec!["alpha", "beta"]);
}

#[test]
fn unknown_package_lists_the_workspace_members() {
    let ws = workspace();
    let err = ProjectInfo::discover(&CargoArgs {
        package: Some("gamma".into()),
        ..args(ws.path())
    })
    .unwrap_err();

    let Error::PackageNotFound { spec, available } = err else {
        panic!("expected PackageNotFound, got {err:?}");
    };
    assert_eq!(spec, "gamma");
    assert!(available.contains(&"alpha".to_string()));
}

#[test]
fn ambiguous_binary_without_default_run_requires_bin_flag() {
    let ws = workspace();
    // Drop `default-run` so `alpha`'s two binaries become ambiguous.
    let manifest = ws.path().join("alpha/Cargo.toml");
    let stripped = fs::read_to_string(&manifest)
        .unwrap()
        .replace("default-run = \"alpha\"\n", "");
    fs::write(&manifest, stripped).unwrap();

    let err = ProjectInfo::discover(&CargoArgs {
        package: Some("alpha".into()),
        ..args(ws.path())
    })
    .unwrap_err();

    let Error::MultipleBinaryTargets { mut targets } = err else {
        panic!("expected MultipleBinaryTargets, got {err:?}");
    };
    targets.sort();
    assert_eq!(targets, vec!["alpha", "alpha-worker"]);
}

#[test]
fn target_dir_override_wins_over_workspace_metadata() {
    let ws = workspace();
    let info = ProjectInfo::discover(&CargoArgs {
        package: Some("beta".into()),
        target_dir: Some(PathBuf::from("/tmp/cargo-serve-out")),
        ..args(ws.path())
    })
    .unwrap();

    assert_eq!(info.target_dir, PathBuf::from("/tmp/cargo-serve-out"));
}

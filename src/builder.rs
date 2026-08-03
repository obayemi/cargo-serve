use std::path::PathBuf;
use std::process::Stdio;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tracing::{debug, info, warn};

use crate::cli::{CargoArgs, ServeArgs};
use crate::error::{Error, Result};
use crate::project::{ProjectInfo, TargetKindSelector};

/// Run the check → build pipeline and return the path to the staged binary.
pub async fn build(project: &ProjectInfo, args: &ServeArgs) -> Result<PathBuf> {
    let show_logs = !args.no_build_logs;

    if !args.no_check {
        info!("Running cargo check...");
        if !run_cargo_check(project, args, show_logs).await? {
            return Err(Error::CheckFailed);
        }
        info!("Check passed");
    }

    info!("Running cargo build...");
    let binary_path = run_cargo_build(project, args, show_logs).await?;
    info!("Build succeeded");

    let staged = stage_binary(&binary_path, project)?;
    debug!(staged = %staged.display(), "Binary staged");

    Ok(staged)
}

async fn run_cargo_check(project: &ProjectInfo, args: &ServeArgs, show_logs: bool) -> Result<bool> {
    let mut cmd = cargo_command("check", project, args);

    cmd.stdout(Stdio::piped())
        .stderr(if show_logs {
            Stdio::inherit()
        } else {
            Stdio::null()
        })
        .kill_on_drop(true);

    let mut child = cmd.spawn()?;

    // Drain stdout so the process doesn't block
    if let Some(stdout) = child.stdout.take() {
        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if show_logs {
                print_diagnostic(&line);
            }
        }
    }

    let status = child.wait().await?;
    Ok(status.success())
}

async fn run_cargo_build(
    project: &ProjectInfo,
    args: &ServeArgs,
    show_logs: bool,
) -> Result<PathBuf> {
    let mut cmd = cargo_command("build", project, args);

    cmd.stdout(Stdio::piped())
        .stderr(if show_logs {
            Stdio::inherit()
        } else {
            Stdio::null()
        })
        .kill_on_drop(true);

    let mut child = cmd.spawn()?;
    let mut binary_path: Option<PathBuf> = None;

    if let Some(stdout) = child.stdout.take() {
        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(path) =
                parse_compiler_artifact(&line, &project.target_name, project.target_kind)
            {
                binary_path = Some(path);
            }
            if show_logs {
                print_diagnostic(&line);
            }
        }
    }

    let status = child.wait().await?;
    if !status.success() {
        return Err(Error::BuildFailed);
    }

    // Fall back to the expected binary path if JSON parsing didn't find it
    let binary_path = match binary_path {
        Some(p) => p,
        None => {
            let fallback = expected_binary_path(project, &args.cargo);
            warn!(
                path = %fallback.display(),
                "Binary not found in cargo JSON output, using expected path"
            );
            fallback
        }
    };

    if !binary_path.exists() {
        return Err(Error::BinaryNotFoundInOutput);
    }

    Ok(binary_path)
}

/// Print a cargo diagnostic message (compiler-message with rendered output).
fn print_diagnostic(json_line: &str) {
    let Ok(msg) = serde_json::from_str::<serde_json::Value>(json_line) else {
        return;
    };
    if msg.get("reason").and_then(|r| r.as_str()) != Some("compiler-message") {
        return;
    }
    if let Some(rendered) = msg
        .get("message")
        .and_then(|m| m.get("rendered"))
        .and_then(|r| r.as_str())
    {
        eprint!("{rendered}");
    }
}

/// Build a `cargo <subcommand>` invocation for the selected target, carrying
/// every forwarded `cargo run`-style flag.
fn cargo_command(subcommand: &str, project: &ProjectInfo, args: &ServeArgs) -> Command {
    let mut cmd = Command::new("cargo");
    cmd.arg(subcommand)
        .arg(project.target_kind.cargo_flag())
        .arg(&project.target_name)
        .arg("--package")
        .arg(&project.package_name)
        .arg("--message-format=json")
        .args(args.cargo.forward_flags());
    cmd
}

/// Construct the expected artifact path from project metadata.
///
/// Mirrors cargo's layout: `<target-dir>[/<triple>]/<profile>[/examples]/<name>`.
fn expected_binary_path(project: &ProjectInfo, args: &CargoArgs) -> PathBuf {
    let mut path = project.target_dir.clone();
    if let Some(triple) = &args.target {
        path.push(triple);
    }
    path.push(args.profile_dir());
    if let Some(subdir) = project.target_kind.artifact_subdir() {
        path.push(subdir);
    }
    path.push(&project.target_name);
    path
}

/// Parse a JSON compiler artifact message and extract the executable path.
pub fn parse_compiler_artifact(
    json_line: &str,
    target_name: &str,
    target_kind: TargetKindSelector,
) -> Option<PathBuf> {
    let msg: serde_json::Value = serde_json::from_str(json_line).ok()?;

    if msg.get("reason")?.as_str()? != "compiler-artifact" {
        return None;
    }

    let target = msg.get("target")?;
    let name = target.get("name")?.as_str()?;
    let kind = target.get("kind")?.as_array()?;

    if name != target_name
        || !kind
            .iter()
            .any(|k| k.as_str() == Some(target_kind.artifact_kind()))
    {
        return None;
    }

    let executable = msg.get("executable")?.as_str()?;
    Some(PathBuf::from(executable))
}

fn stage_binary(source: &PathBuf, project: &ProjectInfo) -> Result<PathBuf> {
    let staging_dir = project.staging_dir();
    std::fs::create_dir_all(&staging_dir).map_err(|e| Error::StagingFailed {
        path: staging_dir.clone(),
        source: e,
    })?;

    let dest = project.staged_binary();

    // Copy to a temp file in the same directory, then atomically rename it over
    // the destination. A plain copy truncates `dest` in place, which fails with
    // ETXTBSY (Linux) when the previous staged binary is still being executed by
    // the running server. rename(2) swaps the directory entry to a fresh inode,
    // leaving the running process's already-open inode untouched.
    let tmp = staging_dir.join(staging_tmp_name(&project.target_name, std::process::id()));

    let staged = copy_and_swap(source, &tmp, &dest);
    if staged.is_err() {
        // Don't leave a half-staged temp file behind on failure.
        let _ = std::fs::remove_file(&tmp);
    }
    staged
}

/// Name of the per-process staging temp file.
///
/// The pid keeps concurrent `cargo-serve` instances watching the same project
/// from sharing a temp file: without it they race on `rename`, and whichever
/// renames second finds its temp already gone and fails with `ENOENT`.
fn staging_tmp_name(bin_name: &str, pid: u32) -> String {
    format!(".{bin_name}.{pid}.new")
}

/// Copy `source` to `tmp`, make it executable, then atomically rename it onto `dest`.
fn copy_and_swap(source: &PathBuf, tmp: &PathBuf, dest: &PathBuf) -> Result<PathBuf> {
    let stage = |e| Error::StagingFailed {
        path: tmp.clone(),
        source: e,
    };
    std::fs::copy(source, tmp).map_err(stage)?;

    // Ensure the staged binary is executable.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(tmp).map_err(stage)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(tmp, perms).map_err(stage)?;
    }

    std::fs::rename(tmp, dest).map_err(|e| Error::StagingFailed {
        path: dest.clone(),
        source: e,
    })?;

    Ok(dest.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staging_tmp_name_is_unique_per_process() {
        // Two cargo-serve instances watching the same project must not share a
        // staging temp file, or they race on rename and one gets ENOENT.
        assert_ne!(
            staging_tmp_name("app", 100),
            staging_tmp_name("app", 200),
            "different processes must use distinct temp files"
        );
    }

    #[test]
    fn staging_tmp_name_is_hidden_and_distinct_from_dest() {
        let name = staging_tmp_name("app", 42);
        assert!(name.starts_with('.'), "temp file should be hidden");
        assert_ne!(name, "app", "temp must not collide with the staged binary");
    }

    fn parse_bin(json: &str) -> Option<PathBuf> {
        parse_compiler_artifact(json, "myapp", TargetKindSelector::Bin)
    }

    #[test]
    fn parse_artifact_success() {
        let json = r#"{"reason":"compiler-artifact","package_id":"myapp 0.1.0","manifest_path":"/tmp/myapp/Cargo.toml","target":{"kind":["bin"],"name":"myapp","src_path":"/tmp/myapp/src/main.rs"},"executable":"/tmp/myapp/target/debug/myapp","filenames":["/tmp/myapp/target/debug/myapp"],"fresh":false}"#;
        assert_eq!(
            parse_bin(json),
            Some(PathBuf::from("/tmp/myapp/target/debug/myapp"))
        );
    }

    #[test]
    fn parse_artifact_wrong_name() {
        let json = r#"{"reason":"compiler-artifact","target":{"kind":["bin"],"name":"other"},"executable":"/tmp/other"}"#;
        assert_eq!(parse_bin(json), None);
    }

    #[test]
    fn parse_artifact_lib_target() {
        let json = r#"{"reason":"compiler-artifact","target":{"kind":["lib"],"name":"myapp"},"executable":null}"#;
        assert_eq!(parse_bin(json), None);
    }

    #[test]
    fn parse_non_artifact_message() {
        let json = r#"{"reason":"build-script-executed","package_id":"foo"}"#;
        assert_eq!(parse_bin(json), None);
    }

    #[test]
    fn parse_invalid_json() {
        assert_eq!(parse_bin("not json"), None);
    }

    #[test]
    fn parse_artifact_distinguishes_example_from_binary() {
        // A package can have a binary and an example sharing a name; picking the
        // wrong one would serve the wrong executable.
        let example = r#"{"reason":"compiler-artifact","target":{"kind":["example"],"name":"myapp"},"executable":"/tmp/myapp/target/debug/examples/myapp"}"#;
        assert_eq!(parse_bin(example), None);
        assert_eq!(
            parse_compiler_artifact(example, "myapp", TargetKindSelector::Example),
            Some(PathBuf::from("/tmp/myapp/target/debug/examples/myapp"))
        );
    }

    fn test_project(target_kind: TargetKindSelector) -> ProjectInfo {
        ProjectInfo {
            package_name: "myapp".into(),
            target_name: "myapp".into(),
            target_kind,
            target_dir: PathBuf::from("/tmp/myapp/target"),
            workspace_root: PathBuf::from("/tmp/myapp"),
        }
    }

    #[test]
    fn expected_binary_path_debug() {
        assert_eq!(
            expected_binary_path(
                &test_project(TargetKindSelector::Bin),
                &CargoArgs::default()
            ),
            PathBuf::from("/tmp/myapp/target/debug/myapp")
        );
    }

    #[test]
    fn expected_binary_path_custom_profile() {
        let args = CargoArgs {
            profile: Some("fast-dev".into()),
            ..Default::default()
        };
        assert_eq!(
            expected_binary_path(&test_project(TargetKindSelector::Bin), &args),
            PathBuf::from("/tmp/myapp/target/fast-dev/myapp")
        );
    }

    #[test]
    fn expected_binary_path_cross_compiled_nests_under_the_triple() {
        let args = CargoArgs {
            target: Some("aarch64-unknown-linux-gnu".into()),
            release: true,
            ..Default::default()
        };
        assert_eq!(
            expected_binary_path(&test_project(TargetKindSelector::Bin), &args),
            PathBuf::from("/tmp/myapp/target/aarch64-unknown-linux-gnu/release/myapp")
        );
    }

    #[test]
    fn expected_binary_path_example_nests_under_examples() {
        assert_eq!(
            expected_binary_path(
                &test_project(TargetKindSelector::Example),
                &CargoArgs::default()
            ),
            PathBuf::from("/tmp/myapp/target/debug/examples/myapp")
        );
    }

    #[cfg(unix)]
    #[test]
    fn stage_binary_atomically_replaces_in_use_destination() {
        use std::io::Read;
        use std::os::unix::fs::MetadataExt;

        let tmp = tempfile::tempdir().unwrap();
        let project = ProjectInfo {
            target_dir: tmp.path().join("target"),
            workspace_root: tmp.path().to_path_buf(),
            ..test_project(TargetKindSelector::Bin)
        };

        // First build → first staged binary.
        let src1 = tmp.path().join("src1");
        std::fs::write(&src1, b"VERSION-1").unwrap();
        let dest = stage_binary(&src1, &project).unwrap();
        let inode_1 = std::fs::metadata(&dest).unwrap().ino();

        // Emulate the running server holding the staged binary's inode open.
        // (A running executable keeps its file's inode mapped exactly like this.)
        let mut held = std::fs::File::open(&dest).unwrap();

        // Second build → re-stage over the same path while the first is still in use.
        let src2 = tmp.path().join("src2");
        std::fs::write(&src2, b"VERSION-2-LONGER").unwrap();
        let dest2 = stage_binary(&src2, &project).unwrap();
        assert_eq!(dest, dest2, "staging path must be stable");
        let inode_2 = std::fs::metadata(&dest).unwrap().ino();

        // Atomic replacement must give the destination a FRESH inode rather than
        // truncating the in-use one in place (an in-place overwrite fails with
        // ETXTBSY when the server is executing it).
        assert_ne!(
            inode_1, inode_2,
            "re-staging must not overwrite the in-use inode in place"
        );

        // The still-open handle keeps seeing the original content (old inode intact).
        let mut old = String::new();
        held.read_to_string(&mut old).unwrap();
        assert_eq!(old, "VERSION-1");

        // The destination now holds the new content.
        assert_eq!(std::fs::read(&dest).unwrap(), b"VERSION-2-LONGER");
    }

    #[test]
    fn expected_binary_path_release() {
        let args = CargoArgs {
            release: true,
            ..Default::default()
        };
        assert_eq!(
            expected_binary_path(&test_project(TargetKindSelector::Bin), &args),
            PathBuf::from("/tmp/myapp/target/release/myapp")
        );
    }
}

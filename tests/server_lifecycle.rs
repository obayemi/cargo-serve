use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::sync::mpsc;

use cargo_serve::error::Error;
use cargo_serve::server::{Server, ServerExit};

/// Write an executable shell script. `body` is the script after the shebang.
fn write_script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let script = dir.join(name);
    // One short-lived write handle: the longer a descriptor stays open on a file
    // another test thread might exec, the wider the ETXTBSY window (see
    // `spawn_server`).
    std::fs::write(&script, format!("#!/bin/sh\n{body}")).unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    script
}

/// A fake server that runs until killed.
fn create_fake_server(dir: &Path) -> PathBuf {
    write_script(
        dir,
        "fake_server",
        "echo \"STARTED\"\n\
         trap 'echo STOPPED; exit 0' TERM\n\
         while true; do sleep 0.1; done\n",
    )
}

/// A fake server that exits on its own after a short delay.
fn create_self_exiting_server(dir: &Path, code: u8) -> PathBuf {
    write_script(dir, "self_exit", &format!("sleep 0.1\nexit {code}\n"))
}

/// Spawn a server, retrying while the executable is still reported busy.
///
/// These tests run concurrently in one process: each writes a script and execs
/// it. A `fork` on one thread inherits any write descriptor another thread holds
/// open, and `exec` refuses a file that is open for writing (`ETXTBSY`), so the
/// spawn can fail for reasons that have nothing to do with the code under test.
/// The descriptor is gone as soon as that child execs, so a brief retry removes
/// the race without weakening the assertion.
fn spawn_server(binary: &Path, args: &[String], tx: mpsc::UnboundedSender<ServerExit>) -> Server {
    for _ in 0..50 {
        match Server::spawn(binary, args, true, tx.clone()) {
            Err(Error::SpawnFailed(e)) if e.kind() == std::io::ErrorKind::ExecutableFileBusy => {
                std::thread::sleep(Duration::from_millis(10));
            }
            other => return other.expect("should spawn"),
        }
    }
    panic!("executable stayed busy: {}", binary.display());
}

#[tokio::test]
async fn spawn_and_stop_server() {
    let tmp = tempfile::tempdir().unwrap();
    let binary = create_fake_server(tmp.path());
    let (tx, _rx) = mpsc::unbounded_channel();

    let server = spawn_server(&binary, &[], tx);

    // Give it a moment to start
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Stop should succeed without error
    server.stop().await.expect("should stop cleanly");
}

#[tokio::test]
async fn spawn_with_args() {
    let tmp = tempfile::tempdir().unwrap();
    // Exits on its own shortly after echoing, so `stop` doesn't hang.
    let script = write_script(tmp.path(), "echo_args", "echo \"$@\"\nsleep 0.2\n");

    let args = vec!["--port".to_string(), "8080".to_string()];
    let (tx, _rx) = mpsc::unbounded_channel();
    let server = spawn_server(&script, &args, tx);

    tokio::time::sleep(Duration::from_millis(300)).await;
    server.stop().await.expect("should stop cleanly");
}

#[tokio::test]
async fn stop_already_exited_server() {
    let tmp = tempfile::tempdir().unwrap();
    let script = write_script(tmp.path(), "quick_exit", "exit 0\n");

    let (tx, _rx) = mpsc::unbounded_channel();
    let server = spawn_server(&script, &[], tx);

    // Wait for process to exit on its own
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Stop should handle already-exited process gracefully
    server.stop().await.expect("should handle already exited");
}

#[tokio::test]
async fn reports_exit_when_server_stops_on_its_own() {
    let tmp = tempfile::tempdir().unwrap();
    let script = create_self_exiting_server(tmp.path(), 0);
    let (tx, mut rx) = mpsc::unbounded_channel::<ServerExit>();

    let server = spawn_server(&script, &[], tx);
    let pid = server.pid();

    // The server exits on its own after ~100ms; the exit must be reported.
    let exit = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("exit should be reported before timeout")
        .expect("channel should deliver the exit");

    assert_eq!(exit.pid, pid, "reported pid must match the spawned server");
    assert!(
        exit.status.expect("wait should succeed").success(),
        "clean exit should be reported as success"
    );
}

#[tokio::test]
async fn reports_nonzero_exit_for_crashing_server() {
    let tmp = tempfile::tempdir().unwrap();
    let script = create_self_exiting_server(tmp.path(), 1);
    let (tx, mut rx) = mpsc::unbounded_channel::<ServerExit>();

    let server = spawn_server(&script, &[], tx);
    let pid = server.pid();

    let exit = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("exit should be reported before timeout")
        .expect("channel should deliver the exit");

    assert_eq!(exit.pid, pid);
    assert!(
        !exit.status.expect("wait should succeed").success(),
        "non-zero exit should be reported as failure"
    );
}

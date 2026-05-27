use std::io::Write;
use std::path::Path;
use std::time::Duration;

use tokio::sync::mpsc;

use cargo_serve::server::{Server, ServerExit};

/// Create a small shell script that acts as a fake server (runs until killed).
fn create_fake_server(dir: &Path) -> std::path::PathBuf {
    let script = dir.join("fake_server");
    let mut f = std::fs::File::create(&script).unwrap();
    writeln!(f, "#!/bin/sh").unwrap();
    writeln!(f, "echo \"STARTED\"").unwrap();
    writeln!(f, "trap 'echo STOPPED; exit 0' TERM").unwrap();
    writeln!(f, "while true; do sleep 0.1; done").unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    script
}

/// Create a script that exits on its own after a short delay.
fn create_self_exiting_server(dir: &Path, code: u8) -> std::path::PathBuf {
    let script = dir.join("self_exit");
    let mut f = std::fs::File::create(&script).unwrap();
    writeln!(f, "#!/bin/sh").unwrap();
    writeln!(f, "sleep 0.1").unwrap();
    writeln!(f, "exit {code}").unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    script
}

#[tokio::test]
async fn spawn_and_stop_server() {
    let tmp = tempfile::tempdir().unwrap();
    let binary = create_fake_server(tmp.path());
    let (tx, _rx) = mpsc::unbounded_channel();

    let server = Server::spawn(&binary, &[], true, tx).expect("should spawn fake server");

    // Give it a moment to start
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Stop should succeed without error
    server.stop().await.expect("should stop cleanly");
}

#[tokio::test]
async fn spawn_with_args() {
    let tmp = tempfile::tempdir().unwrap();
    let script = tmp.path().join("echo_args");
    {
        let mut f = std::fs::File::create(&script).unwrap();
        writeln!(f, "#!/bin/sh").unwrap();
        writeln!(f, "echo \"$@\"").unwrap();
        // Exit immediately so stop doesn't hang
        writeln!(f, "sleep 0.2").unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    let args = vec!["--port".to_string(), "8080".to_string()];
    let (tx, _rx) = mpsc::unbounded_channel();
    let server = Server::spawn(&script, &args, true, tx).expect("should spawn");

    tokio::time::sleep(Duration::from_millis(300)).await;
    server.stop().await.expect("should stop cleanly");
}

#[tokio::test]
async fn stop_already_exited_server() {
    let tmp = tempfile::tempdir().unwrap();
    let script = tmp.path().join("quick_exit");
    {
        let mut f = std::fs::File::create(&script).unwrap();
        writeln!(f, "#!/bin/sh").unwrap();
        writeln!(f, "exit 0").unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    let (tx, _rx) = mpsc::unbounded_channel();
    let server = Server::spawn(&script, &[], true, tx).expect("should spawn");

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

    let server = Server::spawn(&script, &[], true, tx).expect("should spawn");
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

    let server = Server::spawn(&script, &[], true, tx).expect("should spawn");
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

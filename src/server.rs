use std::path::Path;
use std::process::{ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::process::Command;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use crate::error::{Error, Result};

/// Monotonic id assigned to each spawned server, used to match an exit
/// notification to the server that produced it (immune to PID reuse).
static NEXT_ID: AtomicU64 = AtomicU64::new(0);

/// Reported on the shared channel when a spawned server process exits — whether
/// it exited on its own (crash, panic, failed startup) or after being stopped.
#[derive(Debug)]
pub struct ServerExit {
    pub id: u64,
    pub pid: u32,
    pub status: std::io::Result<ExitStatus>,
}

/// Manages the lifecycle of the server child process.
pub struct Server {
    id: u64,
    pid: u32,
    /// Background task that owns the child, waits for its exit, reaps it, and
    /// reports the exit on the shared channel. Completes once the child is gone.
    waiter: JoinHandle<()>,
}

/// Timeout before escalating from SIGTERM to SIGKILL.
const SHUTDOWN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

impl Server {
    /// Spawn the server binary with the given arguments.
    ///
    /// When the process exits, a [`ServerExit`] is sent on `exit_tx` so the
    /// supervisor can detect crashes and failed startups.
    pub fn spawn(
        binary: &Path,
        args: &[String],
        show_logs: bool,
        exit_tx: mpsc::UnboundedSender<ServerExit>,
    ) -> Result<Self> {
        info!(binary = %binary.display(), "Starting server");

        let mut cmd = Command::new(binary);
        cmd.args(args);

        if !show_logs {
            cmd.stdout(Stdio::null()).stderr(Stdio::null());
        }

        // Spawn in its own process group so we can signal the whole tree
        #[cfg(unix)]
        unsafe {
            cmd.pre_exec(|| {
                libc::setpgid(0, 0);
                Ok(())
            });
        }

        cmd.kill_on_drop(true);

        let mut child = cmd.spawn().map_err(Error::SpawnFailed)?;
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let pid = child.id().unwrap_or(0);
        info!(pid, "Server started");

        // Own the child in a background task. This lets the supervisor observe
        // the server's exit (crash or failed startup) via `exit_tx` without
        // holding a borrow of the `Server` across its event-loop `select!`. The
        // task also reaps the child, so an exited server never lingers as a
        // zombie.
        let waiter = tokio::spawn(async move {
            let status = child.wait().await;
            match &status {
                Ok(s) => debug!(pid, status = %s, "Server exited"),
                Err(e) => warn!(pid, "Error waiting for server: {e}"),
            }
            let _ = exit_tx.send(ServerExit { id, pid, status });
        });

        Ok(Self { id, pid, waiter })
    }

    /// The unique id of this server instance.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// The OS process id of the server.
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Gracefully stop the server: SIGTERM the process group, then SIGKILL after timeout.
    pub async fn stop(mut self) -> Result<()> {
        let pid = self.pid;

        // Already exited and reaped by the waiter task — nothing to signal.
        if self.waiter.is_finished() {
            debug!(pid, "Server already exited");
            return Ok(());
        }

        info!(pid, "Stopping server");
        signal_process_group(pid, Signal::Term)?;

        if tokio::time::timeout(SHUTDOWN_TIMEOUT, &mut self.waiter)
            .await
            .is_err()
        {
            warn!(pid, "Server did not exit in time, sending SIGKILL");
            signal_process_group(pid, Signal::Kill)?;
            // Bounded: even after SIGKILL we never block the supervisor forever.
            let _ = tokio::time::timeout(SHUTDOWN_TIMEOUT, &mut self.waiter).await;
        }

        Ok(())
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        // Safety net for ungraceful teardown (the supervisor unwinding on a
        // panic, or returning an error before `process::exit`): make sure the
        // server's process group does not outlive us. Skipped when the child has
        // already exited; harmless otherwise (ESRCH is treated as success).
        if !self.waiter.is_finished() {
            let _ = signal_process_group(self.pid, Signal::Kill);
        }
        self.waiter.abort();
    }
}

enum Signal {
    Term,
    Kill,
}

#[cfg(unix)]
fn signal_process_group(pid: u32, signal: Signal) -> Result<()> {
    let sig = match signal {
        Signal::Term => libc::SIGTERM,
        Signal::Kill => libc::SIGKILL,
    };
    // Negative PID signals the entire process group
    let ret = unsafe { libc::kill(-(pid as i32), sig) };
    if ret != 0 {
        let err = std::io::Error::last_os_error();
        // The process group is already gone (the server exited between our
        // liveness check and this signal) — that is the outcome we wanted.
        if err.raw_os_error() == Some(libc::ESRCH) {
            return Ok(());
        }
        return Err(Error::StopFailed(err));
    }
    Ok(())
}

#[cfg(not(unix))]
fn signal_process_group(_pid: u32, _signal: Signal) -> Result<()> {
    // On non-Unix, kill_on_drop handles cleanup
    Ok(())
}

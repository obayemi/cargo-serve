use std::path::Path;

use tokio::process::{Child, Command};
use tracing::{debug, info, warn};

use crate::error::{Error, Result};

/// Manages the lifecycle of the server child process.
pub struct Server {
    child: Child,
}

/// Timeout before escalating from SIGTERM to SIGKILL.
const SHUTDOWN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

impl Server {
    /// Spawn the server binary with the given arguments.
    pub fn spawn(binary: &Path, args: &[String]) -> Result<Self> {
        info!(binary = %binary.display(), "Starting server");

        let mut cmd = Command::new(binary);
        cmd.args(args);

        // Spawn in its own process group so we can signal the whole tree
        #[cfg(unix)]
        unsafe {
            cmd.pre_exec(|| {
                libc::setpgid(0, 0);
                Ok(())
            });
        }

        cmd.kill_on_drop(true);

        let child = cmd.spawn().map_err(Error::SpawnFailed)?;
        info!(pid = child.id().unwrap_or(0), "Server started");

        Ok(Self { child })
    }

    /// Gracefully stop the server: SIGTERM the process group, then SIGKILL after timeout.
    pub async fn stop(mut self) -> Result<()> {
        let pid = self.child.id();

        match pid {
            Some(pid) => {
                info!(pid, "Stopping server");
                signal_process_group(pid, Signal::Term)?;

                tokio::select! {
                    status = self.child.wait() => {
                        match status {
                            Ok(s) => debug!(status = %s, "Server exited"),
                            Err(e) => warn!("Error waiting for server: {e}"),
                        }
                    }
                    _ = tokio::time::sleep(SHUTDOWN_TIMEOUT) => {
                        warn!(pid, "Server did not exit in time, sending SIGKILL");
                        signal_process_group(pid, Signal::Kill)?;
                        let _ = self.child.wait().await;
                    }
                }
            }
            None => {
                debug!("Server already exited");
            }
        }

        Ok(())
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
        return Err(Error::StopFailed(std::io::Error::last_os_error()));
    }
    Ok(())
}

#[cfg(not(unix))]
fn signal_process_group(_pid: u32, _signal: Signal) -> Result<()> {
    // On non-Unix, kill_on_drop handles cleanup
    Ok(())
}

use std::time::Duration;

use clap::Parser;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use cargo_serve::builder;
use cargo_serve::cli::{CargoSubcommand, ServeArgs};
use cargo_serve::error::Result;
use cargo_serve::project::ProjectInfo;
use cargo_serve::server::{Server, ServerExit};
use cargo_serve::watcher;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("cargo_serve=info")),
        )
        .with_target(false)
        .init();

    let CargoSubcommand::Serve(args) = CargoSubcommand::parse();

    if let Err(e) = run(args).await {
        error!("{e}");
        std::process::exit(1);
    }
}

async fn run(args: ServeArgs) -> Result<()> {
    let project = ProjectInfo::discover(args.bin.as_deref())?;
    info!(
        package = %project.package_name,
        bin = %project.bin_name,
        "Discovered project"
    );

    // Initial build — fail hard if it doesn't compile
    let binary = builder::build(&project, &args).await?;

    // Channel on which each spawned server reports its own exit. Owned by this
    // loop so crash/startup-failure detection never borrows `current_server`
    // across the event-loop `select!`.
    let (exit_tx, mut exit_rx) = mpsc::unbounded_channel::<ServerExit>();

    // Start the server. A failure to start the very first server is fatal.
    let mut current_server = Some(Server::spawn(
        &binary,
        &args.server_args,
        !args.no_server_logs,
        exit_tx.clone(),
    )?);

    // Start file watcher
    let debounce = Duration::from_millis(args.debounce_ms);
    let (mut file_events, _watcher) = watcher::start(
        &project.workspace_root,
        debounce,
        &args.extra_watch,
        &args.extra_ignore,
    )?;
    info!("Watching for changes...");

    // Main event loop
    loop {
        tokio::select! {
            // The running server exited on its own (crash, panic, or a failed
            // startup such as a port already in use).
            Some(exit) = exit_rx.recv() => {
                if current_server.as_ref().map(Server::id) == Some(exit.id) {
                    match exit.status {
                        Ok(status) => error!(
                            "Server (pid {}) exited unexpectedly ({status}). Save a change to rebuild and restart.",
                            exit.pid
                        ),
                        Err(e) => error!("Server (pid {}) could not be waited on: {e}", exit.pid),
                    }
                    current_server = None;
                } else {
                    // A previously-stopped server reporting its exit — expected.
                    debug!("Previous server (pid {}) exited", exit.pid);
                }
            }

            // File change event
            Some(()) = file_events.recv() => {
                info!("Change detected, rebuilding...");

                // Build with cancellation: if another change comes during build,
                // the select! will drop this future (killing cargo via kill_on_drop)
                let build_result = tokio::select! {
                    result = builder::build(&project, &args) => Some(result),
                    Some(()) = file_events.recv() => {
                        info!("New change during build, restarting build...");
                        None
                    }
                };

                match build_result {
                    Some(Ok(new_binary)) => {
                        // Build succeeded — restart server
                        if let Some(server) = current_server.take()
                            && let Err(e) = server.stop().await
                        {
                            warn!("Error stopping previous server: {e}");
                        }
                        // A failed restart is recoverable: keep supervising so the
                        // next successful build can bring the server back up.
                        match Server::spawn(
                            &new_binary,
                            &args.server_args,
                            !args.no_server_logs,
                            exit_tx.clone(),
                        ) {
                            Ok(server) => {
                                info!("Server restarted with new binary");
                                current_server = Some(server);
                            }
                            Err(e) => {
                                error!("Failed to start server: {e}. Save a change to retry.");
                                current_server = None;
                            }
                        }
                    }
                    Some(Err(e)) => {
                        error!("Build failed: {e}");
                        info!("Keeping current server running");
                    }
                    None => {
                        // Build was cancelled, loop will pick up the new event
                        continue;
                    }
                }
            }

            // Graceful shutdown on Ctrl+C
            _ = tokio::signal::ctrl_c() => {
                info!("Shutting down...");
                if let Some(server) = current_server.take()
                    && let Err(e) = server.stop().await
                {
                    warn!("Error stopping server: {e}");
                }
                info!("Goodbye");
                break;
            }
        }
    }

    Ok(())
}

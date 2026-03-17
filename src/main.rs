use std::time::Duration;

use clap::Parser;
use tracing::{error, info};

use cargo_serve::builder;
use cargo_serve::cli::{CargoSubcommand, ServeArgs};
use cargo_serve::error::Result;
use cargo_serve::project::ProjectInfo;
use cargo_serve::server::Server;
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

    // Start the server
    let mut current_server = Server::spawn(&binary, &args.server_args)?;

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
                        current_server.stop().await?;
                        current_server = Server::spawn(&new_binary, &args.server_args)?;
                        info!("Server restarted with new binary");
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
                current_server.stop().await?;
                info!("Goodbye");
                break;
            }
        }
    }

    Ok(())
}

use std::path::Path;
use std::time::Duration;

use ignore::gitignore::GitignoreBuilder;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;
use tracing::{debug, trace, warn};

use crate::error::{Result, chain};

/// Start a file watcher that sends debounced change notifications on the returned channel.
///
/// Watches the workspace root recursively, respecting .gitignore, and ignoring
/// the `target/` directory.
pub fn start(
    workspace_root: &Path,
    debounce: Duration,
    extra_watch: &[String],
    extra_ignore: &[String],
) -> Result<(mpsc::Receiver<()>, RecommendedWatcher)> {
    let (tx, rx) = mpsc::channel::<()>(1);

    // Build gitignore matcher for the workspace
    let mut gitignore_builder = GitignoreBuilder::new(workspace_root);
    let gitignore_path = workspace_root.join(".gitignore");
    if gitignore_path.exists() {
        gitignore_builder.add(&gitignore_path);
    }
    // Always ignore target directory
    let _ = gitignore_builder.add_line(None, "target/");
    let _ = gitignore_builder.add_line(None, ".jj/");
    let _ = gitignore_builder.add_line(None, ".git/");
    for pattern in extra_ignore {
        let _ = gitignore_builder.add_line(None, pattern);
    }
    let gitignore = gitignore_builder.build().unwrap_or_else(|e| {
        warn!("Failed to build gitignore matcher: {}", chain(&e));
        GitignoreBuilder::new(workspace_root).build().unwrap()
    });

    let debounce_tx = tx.clone();
    let (debounce_trigger_tx, mut debounce_trigger_rx) = mpsc::channel::<()>(16);

    // Debounce task: waits for `debounce` duration of quiet before emitting
    tokio::spawn(async move {
        loop {
            // Wait for first event
            if debounce_trigger_rx.recv().await.is_none() {
                break;
            }

            // Drain further events during the debounce window
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(debounce) => break,
                    result = debounce_trigger_rx.recv() => {
                        if result.is_none() {
                            return;
                        }
                        // Got another event, restart debounce timer
                        continue;
                    }
                }
            }

            // Emit debounced notification
            let _ = debounce_tx.try_send(());
        }
    });

    let workspace = workspace_root.to_path_buf();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        match res {
            Ok(event) => {
                // Only trigger on modifications, creates, and removes
                match event.kind {
                    EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_) => {}
                    _ => return,
                }

                // Check if any path passes the gitignore filter
                let dominated_by_gitignore = event.paths.iter().all(|p| {
                    let is_dir = p.is_dir();
                    let relative = p.strip_prefix(&workspace).unwrap_or(p);
                    gitignore
                        .matched_path_or_any_parents(relative, is_dir)
                        .is_ignore()
                });

                if dominated_by_gitignore {
                    trace!(?event.paths, "Ignored by gitignore");
                    return;
                }

                debug!(?event.paths, "File change detected");
                let _ = debounce_trigger_tx.try_send(());
            }
            Err(e) => warn!("Watch error: {}", chain(&e)),
        }
    })?;

    // Watch workspace root
    watcher.watch(workspace_root, RecursiveMode::Recursive)?;

    // Watch additional paths
    for extra in extra_watch {
        let path = Path::new(extra);
        if path.exists() {
            watcher.watch(path, RecursiveMode::Recursive)?;
            debug!(path = %path.display(), "Watching additional path");
        } else {
            warn!(path = %path.display(), "Extra watch path does not exist, skipping");
        }
    }

    Ok((rx, watcher))
}

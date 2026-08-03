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
/// the build output directory.
pub fn start(
    workspace_root: &Path,
    target_dir: &Path,
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
    // A `--target-dir` pointing somewhere else inside the workspace would
    // otherwise make every build trigger the next one.
    if let Some(pattern) = target_ignore_pattern(workspace_root, target_dir) {
        let _ = gitignore_builder.add_line(None, &pattern);
    }
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

/// Gitignore pattern hiding the build output directory, when it sits inside the
/// watched workspace. A target dir outside the workspace never fires events, so
/// it needs no pattern.
fn target_ignore_pattern(workspace_root: &Path, target_dir: &Path) -> Option<String> {
    let relative = target_dir.strip_prefix(workspace_root).ok()?;
    let relative = relative.to_str()?;
    if relative.is_empty() {
        return None;
    }
    Some(format!("/{relative}/"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn custom_target_dir_inside_the_workspace_is_ignored() {
        assert_eq!(
            target_ignore_pattern(Path::new("/ws"), Path::new("/ws/build/out")),
            Some("/build/out/".to_string())
        );
    }

    #[test]
    fn target_dir_outside_the_workspace_needs_no_pattern() {
        assert_eq!(
            target_ignore_pattern(Path::new("/ws"), &PathBuf::from("/elsewhere/target")),
            None
        );
    }

    #[test]
    fn target_dir_equal_to_the_workspace_root_is_not_ignored() {
        // Ignoring the root itself would silence every change.
        assert_eq!(
            target_ignore_pattern(Path::new("/ws"), Path::new("/ws")),
            None
        );
    }
}

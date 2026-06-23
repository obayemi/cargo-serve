# Design: `--eager-start` — serve a previously-built binary when the initial build fails

## Problem

Today, `cargo-serve` runs an initial `cargo check` + `cargo build` at startup and
**fails hard** if the project doesn't compile. The user must reach a buildable
state before the server ever comes up. When iterating on a project that is
mid-refactor (and therefore broken), there's no way to get the last working
server running while you fix the build.

## Goal

Add a flag that lets `cargo-serve` start immediately from a previously-built
binary instead of gating the initial start on a successful build. Once running,
the normal watch loop takes over: the next successful build restarts the server
as usual.

## Flag

`--eager-start` (boolean, default off).

- Off (default): unchanged — an initial build failure is fatal.
- On: an initial build failure is non-fatal; cargo-serve falls back to a
  previously-built binary (or waits for the first good build if none exists).

## Behavior

Only the startup path in `main::run` changes. The watch loop is untouched.

1. Run the normal `builder::build`. On success, behavior is identical to today.
2. On failure (`CheckFailed`, `BuildFailed`, or `BinaryNotFoundInOutput`) **with
   `--eager-start`**: locate a fallback binary.
   - Prefer the binary cargo-serve last staged at `target/.cargo-serve/<bin>`.
     It is already ETXTBSY-safe to execute directly (builds rename a fresh inode
     onto this path, leaving a running process's open inode intact).
   - Otherwise use cargo's own output at `target/{debug,release}/<bin>`. This
     copy is **staged first** (via the existing `stage_binary` copy + atomic
     swap) before running, so a subsequent `cargo build` overwriting
     `target/<profile>/<bin>` in place cannot hit the in-use file (ETXTBSY).
3. If a fallback is found: log a warning that the served binary is stale, start
   the server from the staged path, and enter the watch loop normally.
4. If no fallback exists anywhere: log a warning, enter the watch loop with **no
   server running** (`current_server = None`). The first successful build starts
   the server. The event loop already tolerates a `None` server.

Without `--eager-start`, an initial build failure remains fatal exactly as today.

## New unit (`builder.rs`)

```rust
/// Locate a runnable, ETXTBSY-safe fallback binary for `--eager-start`.
///
/// Prefers the last-staged binary (`target/.cargo-serve/<bin>`); falls back to
/// cargo's own `target/{debug,release}/<bin>`, staging that copy so it is safe to
/// execute while later builds overwrite the cargo output. Returns `Ok(None)` when
/// no previously-built binary exists.
pub fn locate_stale_binary(project: &ProjectInfo, args: &ServeArgs) -> Result<Option<PathBuf>>
```

`expected_binary_path` stays private and is reused from within the module.

## CLI (`cli.rs`)

Add to `ServeArgs`:

```rust
/// Start immediately from a previously-built binary instead of waiting for the
/// initial build to succeed (serves a possibly-stale binary).
#[arg(long)]
pub eager_start: bool,
```

## Error handling

No new error variants. The fallback path catches the existing initial-build
errors only when `--eager-start` is set; all other error propagation is unchanged.

## Testing

- `locate_stale_binary` returns the staged path when `target/.cargo-serve/<bin>`
  exists.
- `locate_stale_binary` stages and returns a staged path when only the cargo
  target (`target/<profile>/<bin>`) exists, and the staged file is created.
- `locate_stale_binary` returns `None` when neither exists.
- CLI: `--eager-start` parses to `true`; default is `false` (extend `parse_full`
  and the minimal-parse default assertions).

## Docs

- Add `--eager-start` to the README options table.
- Add a short "Serving a previously-built binary" subsection with an example
  (`cargo serve --eager-start`) explaining the stale-binary fallback and the
  warn-and-wait behavior when nothing has been built yet.

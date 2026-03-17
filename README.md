# cargo-serve

A Cargo plugin that watches for file changes, rebuilds your project, and **only restarts the server when the build succeeds** — keeping the last working server running during broken builds.

## Features

- **Smart restart**: failed builds never take down your running server
- **Check before build**: runs `cargo check` first for fast error feedback, then builds only on success
- **Binary staging**: copies the built binary to a staging area so `cargo build` never overwrites the running process
- **Build cancellation**: new file changes during a build cancel the current build and start fresh
- **Gitignore-aware**: respects `.gitignore` rules automatically
- **Process group management**: cleanly shuts down server and all child processes via SIGTERM/SIGKILL

## Installation

```sh
cargo install --path .
```

## Usage

```sh
cargo serve [OPTIONS] [-- <server args>...]
```

### Options

| Option | Description | Default |
|---|---|---|
| `-b, --bin <NAME>` | Binary target to run | Package's only binary |
| `--release` | Build in release mode | |
| `--features <F,...>` | Activate features (comma-separated) | |
| `--debounce-ms <MS>` | Debounce delay in milliseconds | `500` |
| `--watch <PATH>` | Additional paths to watch | |
| `--ignore <PATH>` | Additional paths to ignore | |
| `--no-check` | Skip `cargo check`, go straight to build | |

### Examples

```sh
# Basic usage — watches and restarts your server
cargo serve

# Pass arguments to the server
cargo serve -- --port 8080

# Release mode with features
cargo serve --release --features metrics

# Faster rebuilds by skipping check
cargo serve --no-check
```

## How It Works

```
File change → debounce → cargo check → cargo build → stage binary → restart server
                              ↓ fail          ↓ fail
                         keep running     keep running
```

1. Discovers the project via `cargo metadata`
2. Runs an initial build (fails hard if it doesn't compile)
3. Copies the binary to `target/.cargo-serve/` and starts the server from there
4. Watches `src/`, `Cargo.toml`, `Cargo.lock` (and any `--watch` paths) for changes
5. On change: check → build → stage → restart (keeping the old server on failure)
6. On Ctrl+C: sends SIGTERM to the server process group, escalates to SIGKILL after 5s

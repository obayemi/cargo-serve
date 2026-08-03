# cargo-serve

A Cargo plugin that watches for file changes, rebuilds your project, and **only restarts the server when the build succeeds** — keeping the last working server running during broken builds.

## Features

- **`cargo run`-compatible flags**: `-p`, `--bin`, `--example`, `--features`, `--profile`, `--target`, `--offline`, … behave as they do with `cargo run`
- **Smart restart**: failed builds never take down your running server
- **Check before build**: runs `cargo check` first for fast error feedback, then builds only on success
- **Atomic binary staging**: stages each build to a separate file and swaps it in via an atomic rename, so a rebuild never tries to overwrite the binary the server is currently executing (which fails with `ETXTBSY` on Linux)
- **Crash detection**: notices when the server exits on its own — a panic, a non-zero exit, or a failed startup such as a port already in use — and reports it instead of pretending the server is still up
- **Resilient restarts**: a failed restart doesn't take down the watcher; the next successful build brings the server back
- **Build cancellation**: new file changes during a build cancel the current build and start fresh
- **Gitignore-aware**: respects `.gitignore` rules automatically
- **Process group management**: cleanly shuts down server and all child processes via SIGTERM/SIGKILL

## Installation

```sh
cargo install --path .
```

### Nix

Run it without installing, or build the package:

```sh
nix run github:obayemi/cargo-serve
nix build github:obayemi/cargo-serve
```

### Home Manager

The flake exposes `overlays.default`, which adds `pkgs.cargo-serve`:

```nix
{
  inputs.cargo-serve.url = "github:obayemi/cargo-serve";

  # in your home-manager configuration
  nixpkgs.overlays = [inputs.cargo-serve.overlays.default];
  home.packages = [pkgs.cargo-serve];
}
```

Or skip the overlay and take the package directly:

```nix
home.packages = [inputs.cargo-serve.packages.${pkgs.system}.cargo-serve];
```

## Usage

```sh
cargo serve [OPTIONS] [-- <server args>...]
```

Run it either as a Cargo subcommand (`cargo serve`) or directly (`cargo-serve`) — `serve` is the default command, so the explicit subcommand is optional when invoking the binary by name.

### Cargo options

These mirror `cargo run`, and are forwarded to the underlying `cargo check` / `cargo build`:

| Option | Description | Default |
|---|---|---|
| `-p, --package <SPEC>` | Package to serve (`name` or `name@version`) | the current package |
| `-b, --bin <NAME>` | Binary target to run | `default-run`, else the package's only binary |
| `--example <NAME>` | Example target to run | |
| `-F, --features <F,...>` | Activate features (comma- or space-separated, repeatable) | |
| `--all-features` | Activate all available features | |
| `--no-default-features` | Do not activate the `default` feature | |
| `-j, --jobs <N>` | Number of parallel jobs | # of CPUs |
| `--keep-going` | Do not abort the build at the first error | |
| `-r, --release` | Build in release mode, with optimizations | |
| `--profile <NAME>` | Build with the given profile | `dev` |
| `--target <TRIPLE>` | Build for the target triple | host |
| `--target-dir <DIR>` | Directory for all generated artifacts | `target/` |
| `--timings[=<FMTS>]` | Output build timing information | |
| `--manifest-path <PATH>` | Path to `Cargo.toml` | discovered from the cwd |
| `--lockfile-path <PATH>` | Path to `Cargo.lock` (unstable) | |
| `--ignore-rust-version` | Ignore `rust-version` in packages | |
| `--locked` / `--offline` / `--frozen` | Lockfile and network rules | |
| `--config <KEY=VALUE>` | Override a configuration value (repeatable) | |
| `-Z <FLAG>` | Unstable (nightly-only) cargo flags | |
| `-v, --verbose` / `-q, --quiet` | Cargo output verbosity (`-vv` for very verbose) | |
| `--color <WHEN>` | `auto`, `always`, or `never` | `auto` |

`--message-format` is not exposed: cargo-serve needs cargo's JSON output to locate the built executable.

### Serve options

| Option | Description | Default |
|---|---|---|
| `--debounce-ms <MS>` | Debounce delay in milliseconds | `500` |
| `--watch <PATH>` | Additional paths to watch | |
| `--ignore <PATH>` | Additional paths to ignore | |
| `--no-check` | Skip `cargo check`, go straight to build | |
| `--no-server-logs` | Hide server stdout/stderr output | |
| `--no-build-logs` | Hide build/check output | |

### Examples

```sh
# Basic usage — watches and restarts your server
cargo serve

# Pass arguments to the server
cargo serve -- --port 8080

# A workspace member, with features, plus arguments for the server
cargo serve -p my-api --features embed-ui -- --seed-rules

# Release mode with features
cargo serve --release --features metrics

# A specific binary, or an example
cargo serve --bin worker
cargo serve --example playground

# A custom profile and an out-of-tree target directory
cargo serve --profile fast-dev --target-dir /tmp/serve-out

# Faster rebuilds by skipping check
cargo serve --no-check
```

## How It Works

```
File change → debounce → cargo check → cargo build → stage binary → restart server
                              ↓ fail          ↓ fail
                         keep running     keep running
```

1. Discovers the package and executable target via `cargo metadata` (honouring `-p`, `--bin`, `--example`, `default-run`)
2. Runs an initial build (fails hard if it doesn't compile)
3. Stages the binary into `target/.cargo-serve/` (copy to a temp file, then atomic rename) and starts the server from there
4. Watches `src/`, `Cargo.toml`, `Cargo.lock` (and any `--watch` paths) for changes, ignoring the build output directory even when `--target-dir` moves it
5. On change: check → build → stage → restart (keeping the old server on failure)
6. If the server exits on its own, logs the exit and waits for the next change to rebuild and restart it
7. On Ctrl+C: sends SIGTERM to the server process group, escalates to SIGKILL after 5s

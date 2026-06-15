use std::ffi::OsString;

use clap::Parser;

/// Dev server with smart restart — keeps the last working server running during broken builds.
#[derive(Debug, Default, Parser)]
#[command(name = "cargo-serve", bin_name = "cargo serve", version, about)]
pub struct ServeArgs {
    /// Binary target to run (defaults to `default-run`, else the package's only binary)
    #[arg(short, long)]
    pub bin: Option<String>,

    /// Build in release mode
    #[arg(long)]
    pub release: bool,

    /// Activate features (comma-separated)
    #[arg(long, value_delimiter = ',')]
    pub features: Vec<String>,

    /// Debounce delay in milliseconds
    #[arg(long, default_value = "500")]
    pub debounce_ms: u64,

    /// Additional paths to watch
    #[arg(long = "watch")]
    pub extra_watch: Vec<String>,

    /// Additional paths to ignore
    #[arg(long = "ignore")]
    pub extra_ignore: Vec<String>,

    /// Skip cargo check, go straight to build
    #[arg(long)]
    pub no_check: bool,

    /// Hide server stdout/stderr output
    #[arg(long)]
    pub no_server_logs: bool,

    /// Hide build/check output
    #[arg(long)]
    pub no_build_logs: bool,

    /// Arguments to pass to the server binary
    #[arg(last = true)]
    pub server_args: Vec<String>,
}

impl ServeArgs {
    /// Parse arguments from the process environment, treating `serve` as the
    /// default (and only) command.
    pub fn parse_env() -> Self {
        Self::parse_from(strip_serve_subcommand(std::env::args_os()))
    }
}

/// Drop the leading `serve` token that cargo injects when the tool is invoked
/// as `cargo serve`. This makes `serve` the implicit default command, so the
/// binary behaves identically whether run as `cargo serve …` or directly as
/// `cargo-serve …`.
fn strip_serve_subcommand(args: impl IntoIterator<Item = OsString>) -> Vec<OsString> {
    let mut args: Vec<OsString> = args.into_iter().collect();
    if args.get(1).is_some_and(|a| a == "serve") {
        args.remove(1);
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(parts: &[&str]) -> Vec<OsString> {
        parts.iter().map(OsString::from).collect()
    }

    #[test]
    fn strips_cargo_injected_serve_subcommand() {
        assert_eq!(
            strip_serve_subcommand(os(&["cargo-serve", "serve", "--release"])),
            os(&["cargo-serve", "--release"])
        );
    }

    #[test]
    fn leaves_direct_invocation_untouched() {
        assert_eq!(
            strip_serve_subcommand(os(&["cargo-serve", "--release"])),
            os(&["cargo-serve", "--release"])
        );
    }

    #[test]
    fn only_strips_the_leading_serve_not_trailing_server_args() {
        assert_eq!(
            strip_serve_subcommand(os(&["cargo-serve", "serve", "--", "serve"])),
            os(&["cargo-serve", "--", "serve"])
        );
    }

    #[test]
    fn parse_minimal_via_cargo_subcommand() {
        let args = ServeArgs::parse_from(strip_serve_subcommand(os(&["cargo-serve", "serve"])));
        assert_eq!(args.bin, None);
        assert!(!args.release);
        assert_eq!(args.debounce_ms, 500);
        assert!(!args.no_check);
        assert!(!args.no_server_logs);
        assert!(!args.no_build_logs);
    }

    #[test]
    fn parse_minimal_via_direct_invocation() {
        let args = ServeArgs::parse_from(strip_serve_subcommand(os(&["cargo-serve"])));
        assert_eq!(args.bin, None);
        assert!(!args.release);
    }

    #[test]
    fn parse_full() {
        let args = ServeArgs::parse_from(strip_serve_subcommand(os(&[
            "cargo-serve",
            "serve",
            "--bin",
            "myapp",
            "--release",
            "--features",
            "feat1,feat2",
            "--debounce-ms",
            "200",
            "--watch",
            "templates",
            "--ignore",
            "docs",
            "--no-check",
            "--no-server-logs",
            "--no-build-logs",
            "--",
            "--port",
            "8080",
        ])));
        assert_eq!(args.bin.as_deref(), Some("myapp"));
        assert!(args.release);
        assert_eq!(args.features, vec!["feat1", "feat2"]);
        assert_eq!(args.debounce_ms, 200);
        assert_eq!(args.extra_watch, vec!["templates"]);
        assert_eq!(args.extra_ignore, vec!["docs"]);
        assert!(args.no_check);
        assert!(args.no_server_logs);
        assert!(args.no_build_logs);
        assert_eq!(args.server_args, vec!["--port", "8080"]);
    }
}

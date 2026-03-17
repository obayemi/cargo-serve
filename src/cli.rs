use clap::Parser;

/// Wrapper for `cargo serve` subcommand pattern.
#[derive(Debug, Parser)]
#[command(name = "cargo", bin_name = "cargo")]
pub enum CargoSubcommand {
    /// Dev server with smart restart — keeps the last working server running during broken builds.
    Serve(ServeArgs),
}

#[derive(Debug, Parser)]
#[command(version, about)]
pub struct ServeArgs {
    /// Binary target to run (defaults to the package's only binary)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal() {
        let CargoSubcommand::Serve(args) = CargoSubcommand::parse_from(["cargo", "serve"]);
        assert_eq!(args.bin, None);
        assert!(!args.release);
        assert_eq!(args.debounce_ms, 500);
        assert!(!args.no_check);
        assert!(!args.no_server_logs);
        assert!(!args.no_build_logs);
    }

    #[test]
    fn parse_full() {
        let CargoSubcommand::Serve(args) = CargoSubcommand::parse_from([
            "cargo",
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
        ]);
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

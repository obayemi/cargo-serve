use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

use clap::{ArgAction, Args, Parser};

/// Dev server with smart restart — keeps the last working server running during broken builds.
#[derive(Debug, Default, Parser)]
#[command(name = "cargo-serve", bin_name = "cargo serve", version, about)]
pub struct ServeArgs {
    #[command(flatten)]
    pub cargo: CargoArgs,

    /// Debounce delay in milliseconds
    #[arg(long, default_value = "500", help_heading = "Serve options")]
    pub debounce_ms: u64,

    /// Additional paths to watch
    #[arg(long = "watch", value_name = "PATH", help_heading = "Serve options")]
    pub extra_watch: Vec<String>,

    /// Additional paths to ignore
    #[arg(long = "ignore", value_name = "PATH", help_heading = "Serve options")]
    pub extra_ignore: Vec<String>,

    /// Skip cargo check, go straight to build
    #[arg(long, help_heading = "Serve options")]
    pub no_check: bool,

    /// Hide server stdout/stderr output
    #[arg(long, help_heading = "Serve options")]
    pub no_server_logs: bool,

    /// Hide build/check output
    #[arg(long, help_heading = "Serve options")]
    pub no_build_logs: bool,

    /// Start immediately from a previously-built binary instead of waiting for
    /// the initial build to succeed (serves a possibly-stale binary)
    #[arg(long)]
    pub eager_start: bool,

    /// Arguments to pass to the server binary
    #[arg(last = true)]
    pub server_args: Vec<String>,
}

/// The subset of `cargo run` flags that select what to build and how, forwarded
/// verbatim to the underlying `cargo check` / `cargo build` invocations.
///
/// `--message-format` is deliberately absent: cargo-serve needs the JSON format
/// to locate the built executable.
#[derive(Debug, Default, Args)]
pub struct CargoArgs {
    /// Package to serve (defaults to the current package)
    #[arg(
        short = 'p',
        long = "package",
        value_name = "SPEC",
        help_heading = "Package selection"
    )]
    pub package: Option<String>,

    /// Binary target to run (defaults to `default-run`, else the package's only binary)
    #[arg(short, long, value_name = "NAME", help_heading = "Target selection")]
    pub bin: Option<String>,

    /// Example target to run
    #[arg(
        long,
        value_name = "NAME",
        conflicts_with = "bin",
        help_heading = "Target selection"
    )]
    pub example: Option<String>,

    /// Activate features (comma- or space-separated, repeatable)
    #[arg(
        short = 'F',
        long,
        value_delimiter = ',',
        value_name = "FEATURES",
        help_heading = "Feature selection"
    )]
    pub features: Vec<String>,

    /// Activate all available features
    #[arg(long, help_heading = "Feature selection")]
    pub all_features: bool,

    /// Do not activate the `default` feature
    #[arg(long, help_heading = "Feature selection")]
    pub no_default_features: bool,

    /// Number of parallel jobs, defaults to # of CPUs
    #[arg(
        short = 'j',
        long,
        value_name = "N",
        allow_hyphen_values = true,
        help_heading = "Compilation options"
    )]
    pub jobs: Option<String>,

    /// Do not abort the build as soon as there is an error
    #[arg(long, help_heading = "Compilation options")]
    pub keep_going: bool,

    /// Build in release mode, with optimizations
    #[arg(short = 'r', long, help_heading = "Compilation options")]
    pub release: bool,

    /// Build with the given profile
    #[arg(
        long,
        value_name = "PROFILE-NAME",
        conflicts_with = "release",
        help_heading = "Compilation options"
    )]
    pub profile: Option<String>,

    /// Build for the target triple
    #[arg(long, value_name = "TRIPLE", help_heading = "Compilation options")]
    pub target: Option<String>,

    /// Directory for all generated artifacts
    #[arg(long, value_name = "DIRECTORY", help_heading = "Compilation options")]
    pub target_dir: Option<PathBuf>,

    /// Output build timing information
    #[arg(
        long,
        value_name = "FMTS",
        num_args = 0..=1,
        require_equals = true,
        default_missing_value = "html",
        help_heading = "Compilation options"
    )]
    pub timings: Option<String>,

    /// Path to Cargo.toml
    #[arg(long, value_name = "PATH", help_heading = "Manifest options")]
    pub manifest_path: Option<PathBuf>,

    /// Path to Cargo.lock (unstable)
    #[arg(long, value_name = "PATH", help_heading = "Manifest options")]
    pub lockfile_path: Option<PathBuf>,

    /// Ignore `rust-version` specification in packages
    #[arg(long, help_heading = "Manifest options")]
    pub ignore_rust_version: bool,

    /// Assert that `Cargo.lock` will remain unchanged
    #[arg(long, help_heading = "Manifest options")]
    pub locked: bool,

    /// Run without accessing the network
    #[arg(long, help_heading = "Manifest options")]
    pub offline: bool,

    /// Equivalent to specifying both --locked and --offline
    #[arg(long, help_heading = "Manifest options")]
    pub frozen: bool,

    /// Use verbose output (-vv for very verbose)
    #[arg(
        short = 'v',
        long,
        action = ArgAction::Count,
        conflicts_with = "quiet",
        help_heading = "Display options"
    )]
    pub verbose: u8,

    /// Do not print cargo log messages
    #[arg(short = 'q', long, help_heading = "Display options")]
    pub quiet: bool,

    /// Control when colored output is used
    #[arg(
        long,
        value_name = "WHEN",
        value_parser = ["auto", "always", "never"],
        help_heading = "Display options"
    )]
    pub color: Option<String>,

    /// Override a configuration value
    #[arg(long, value_name = "KEY=VALUE", help_heading = "Manifest options")]
    pub config: Vec<String>,

    /// Unstable (nightly-only) flags to cargo
    #[arg(short = 'Z', value_name = "FLAG", help_heading = "Manifest options")]
    pub unstable: Vec<String>,
}

impl ServeArgs {
    /// Parse arguments from the process environment, treating `serve` as the
    /// default (and only) command.
    pub fn parse_env() -> Self {
        Self::parse_from(strip_serve_subcommand(std::env::args_os()))
    }
}

impl CargoArgs {
    /// The requested features, flattened across repetitions and accepting both
    /// cargo spellings: `--features a,b` and `--features "a b"`.
    pub fn feature_list(&self) -> Vec<&str> {
        self.features
            .iter()
            .flat_map(|f| f.split([',', ' ']))
            .filter(|f| !f.is_empty())
            .collect()
    }

    /// Flags to forward to `cargo check` / `cargo build`.
    ///
    /// Target selection (`--bin` / `--example`) is *not* included: the target is
    /// resolved against package metadata first, then passed explicitly.
    pub fn forward_flags(&self) -> Vec<OsString> {
        let mut flags = Flags::default();

        let features = self.feature_list();
        if !features.is_empty() {
            flags.value("--features", features.join(","));
        }
        flags.toggle("--all-features", self.all_features);
        flags.toggle("--no-default-features", self.no_default_features);

        if let Some(jobs) = &self.jobs {
            flags.value("--jobs", jobs);
        }
        flags.toggle("--keep-going", self.keep_going);
        flags.toggle("--release", self.release);
        if let Some(profile) = &self.profile {
            flags.value("--profile", profile);
        }
        if let Some(target) = &self.target {
            flags.value("--target", target);
        }
        if let Some(dir) = &self.target_dir {
            flags.value("--target-dir", dir);
        }
        if let Some(fmts) = &self.timings {
            flags.flag(&format!("--timings={fmts}"));
        }
        flags.toggle("--ignore-rust-version", self.ignore_rust_version);

        flags.0.extend(self.metadata_flags());

        for _ in 0..self.verbose {
            flags.flag("-v");
        }
        flags.toggle("--quiet", self.quiet);
        if let Some(color) = &self.color {
            flags.value("--color", color);
        }

        flags.0
    }

    /// Flags that describe *where the manifest lives* and how cargo may touch
    /// the network or lockfile. Passed to `cargo metadata` as well as the build
    /// commands, so both resolve the same workspace the same way.
    ///
    /// Kept to options `cargo metadata` actually accepts — notably it has no
    /// `--ignore-rust-version`, which `forward_flags` adds on its own.
    pub fn metadata_flags(&self) -> Vec<OsString> {
        let mut flags = Flags::default();

        if let Some(path) = &self.manifest_path {
            flags.value("--manifest-path", path);
        }
        if let Some(path) = &self.lockfile_path {
            flags.value("--lockfile-path", path);
        }
        flags.toggle("--locked", self.locked);
        flags.toggle("--offline", self.offline);
        flags.toggle("--frozen", self.frozen);
        for value in &self.config {
            flags.value("--config", value);
        }
        for value in &self.unstable {
            flags.value("-Z", value);
        }

        flags.0
    }

    /// Name of the `target/` subdirectory cargo writes artifacts to for the
    /// selected profile. Built-in profiles don't map one-to-one onto their
    /// directory names: `dev`/`test` land in `debug`, `bench` in `release`.
    pub fn profile_dir(&self) -> &str {
        match self.profile.as_deref() {
            Some("dev" | "test") => "debug",
            Some("bench") => "release",
            Some(custom) => custom,
            None if self.release => "release",
            None => "debug",
        }
    }
}

/// Accumulator for a cargo command line, so each forwarded option reads as one
/// line at the call site.
#[derive(Default)]
struct Flags(Vec<OsString>);

impl Flags {
    fn flag(&mut self, name: &str) {
        self.0.push(OsString::from(name));
    }

    fn toggle(&mut self, name: &str, enabled: bool) {
        if enabled {
            self.flag(name);
        }
    }

    fn value(&mut self, name: &str, value: impl AsRef<OsStr>) {
        self.flag(name);
        self.0.push(value.as_ref().to_os_string());
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

    fn parse(parts: &[&str]) -> ServeArgs {
        ServeArgs::parse_from(strip_serve_subcommand(os(parts)))
    }

    fn try_parse(parts: &[&str]) -> Result<ServeArgs, clap::Error> {
        ServeArgs::try_parse_from(strip_serve_subcommand(os(parts)))
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
        let args = parse(&["cargo-serve", "serve"]);
        assert_eq!(args.cargo.bin, None);
        assert!(!args.cargo.release);
        assert_eq!(args.debounce_ms, 500);
        assert!(!args.no_check);
        assert!(!args.no_server_logs);
        assert!(!args.no_build_logs);
        assert!(!args.eager_start);
    }

    #[test]
    fn parse_minimal_via_direct_invocation() {
        let args = parse(&["cargo-serve"]);
        assert_eq!(args.cargo.bin, None);
        assert!(!args.cargo.release);
    }

    #[test]
    fn parse_full() {
        let args = parse(&[
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
            "--eager-start",
            "--",
            "--port",
            "8080",
        ]);
        assert_eq!(args.cargo.bin.as_deref(), Some("myapp"));
        assert!(args.cargo.release);
        assert_eq!(args.cargo.feature_list(), vec!["feat1", "feat2"]);
        assert_eq!(args.debounce_ms, 200);
        assert_eq!(args.extra_watch, vec!["templates"]);
        assert_eq!(args.extra_ignore, vec!["docs"]);
        assert!(args.no_check);
        assert!(args.no_server_logs);
        assert!(args.no_build_logs);
        assert!(args.eager_start);
        assert_eq!(args.server_args, vec!["--port", "8080"]);
    }

    /// The reported failure: `cargo serve -p pkg --features f -- --flag` was
    /// rejected because `-p` had no equivalent here.
    #[test]
    fn parse_cargo_run_style_package_and_features() {
        let args = parse(&[
            "cargo-serve",
            "serve",
            "-p",
            "fdd-demo",
            "--features",
            "embed-ui",
            "--",
            "--seed-rules",
        ]);
        assert_eq!(args.cargo.package.as_deref(), Some("fdd-demo"));
        assert_eq!(args.cargo.feature_list(), vec!["embed-ui"]);
        assert_eq!(args.server_args, vec!["--seed-rules"]);
    }

    #[test]
    fn parse_cargo_short_flags() {
        let args = parse(&["cargo-serve", "-r", "-F", "a", "-j", "4", "-vv"]);
        assert!(args.cargo.release);
        assert_eq!(args.cargo.feature_list(), vec!["a"]);
        assert_eq!(args.cargo.jobs.as_deref(), Some("4"));
        assert_eq!(args.cargo.verbose, 2);
    }

    #[test]
    fn features_accept_space_separated_and_repeated_flags() {
        let args = parse(&["cargo-serve", "-F", "a b", "--features", "c,d"]);
        assert_eq!(args.cargo.feature_list(), vec!["a", "b", "c", "d"]);
    }

    #[test]
    fn release_and_profile_are_mutually_exclusive() {
        assert!(try_parse(&["cargo-serve", "--release", "--profile", "dev"]).is_err());
    }

    #[test]
    fn bin_and_example_are_mutually_exclusive() {
        assert!(try_parse(&["cargo-serve", "--bin", "a", "--example", "b"]).is_err());
    }

    #[test]
    fn timings_defaults_to_html_and_accepts_a_value() {
        assert_eq!(
            parse(&["cargo-serve", "--timings"])
                .cargo
                .timings
                .as_deref(),
            Some("html")
        );
        assert_eq!(
            parse(&["cargo-serve", "--timings=json"])
                .cargo
                .timings
                .as_deref(),
            Some("json")
        );
    }

    #[test]
    fn forward_flags_empty_by_default() {
        assert!(CargoArgs::default().forward_flags().is_empty());
    }

    #[test]
    fn forward_flags_omits_target_selection() {
        // The target is resolved against metadata and passed separately, so
        // forwarding `--bin`/`--example` here would duplicate it.
        let args = parse(&["cargo-serve", "--bin", "myapp"]).cargo;
        assert!(args.forward_flags().is_empty());
    }

    #[test]
    fn forward_flags_covers_every_cargo_option() {
        let args = parse(&[
            "cargo-serve",
            "--features",
            "a,b",
            "--all-features",
            "--no-default-features",
            "--jobs",
            "2",
            "--keep-going",
            "--profile",
            "custom",
            "--target",
            "x86_64-unknown-linux-gnu",
            "--target-dir",
            "/tmp/t",
            "--timings=html",
            "--manifest-path",
            "/tmp/Cargo.toml",
            "--lockfile-path",
            "/tmp/Cargo.lock",
            "--ignore-rust-version",
            "--locked",
            "--offline",
            "--frozen",
            "-vv",
            "--color",
            "never",
            "--config",
            "k=v",
            "-Z",
            "unstable-options",
        ])
        .cargo;

        assert_eq!(
            args.forward_flags(),
            os(&[
                "--features",
                "a,b",
                "--all-features",
                "--no-default-features",
                "--jobs",
                "2",
                "--keep-going",
                "--profile",
                "custom",
                "--target",
                "x86_64-unknown-linux-gnu",
                "--target-dir",
                "/tmp/t",
                "--timings=html",
                "--ignore-rust-version",
                "--manifest-path",
                "/tmp/Cargo.toml",
                "--lockfile-path",
                "/tmp/Cargo.lock",
                "--locked",
                "--offline",
                "--frozen",
                "--config",
                "k=v",
                "-Z",
                "unstable-options",
                "-v",
                "-v",
                "--color",
                "never",
            ])
        );
    }

    #[test]
    fn metadata_flags_are_shared_with_the_build() {
        // `cargo metadata` must resolve the same workspace under the same
        // network/lockfile rules as the build, or the two disagree.
        let args = parse(&[
            "cargo-serve",
            "--manifest-path",
            "/tmp/Cargo.toml",
            "--offline",
            "--config",
            "k=v",
            "-Z",
            "z",
        ])
        .cargo;
        assert_eq!(
            args.metadata_flags(),
            os(&[
                "--manifest-path",
                "/tmp/Cargo.toml",
                "--offline",
                "--config",
                "k=v",
                "-Z",
                "z",
            ])
        );
    }

    #[test]
    fn metadata_flags_exclude_options_cargo_metadata_rejects() {
        // `cargo metadata` has no --ignore-rust-version; forwarding it there
        // would make discovery fail outright.
        let args = parse(&["cargo-serve", "--ignore-rust-version"]).cargo;
        assert!(args.metadata_flags().is_empty());
        assert_eq!(args.forward_flags(), os(&["--ignore-rust-version"]));
    }

    #[test]
    fn quiet_is_forwarded() {
        let args = parse(&["cargo-serve", "--quiet"]).cargo;
        assert_eq!(args.forward_flags(), os(&["--quiet"]));
    }

    #[test]
    fn profile_dir_maps_builtin_profiles_to_their_directories() {
        let dir = |parts: &[&str]| {
            let mut argv = vec!["cargo-serve"];
            argv.extend_from_slice(parts);
            parse(&argv).cargo.profile_dir().to_string()
        };
        assert_eq!(dir(&[]), "debug");
        assert_eq!(dir(&["--release"]), "release");
        assert_eq!(dir(&["--profile", "dev"]), "debug");
        assert_eq!(dir(&["--profile", "test"]), "debug");
        assert_eq!(dir(&["--profile", "bench"]), "release");
        assert_eq!(dir(&["--profile", "release"]), "release");
        assert_eq!(dir(&["--profile", "fast-dev"]), "fast-dev");
    }
}

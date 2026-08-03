{
  description = "A Cargo plugin that watches, rebuilds, and only restarts your server when the build succeeds";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = {
    self,
    nixpkgs,
    rust-overlay,
    flake-utils,
  }: let
    cargoToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);

    mkCargoServe = pkgs:
      pkgs.rustPlatform.buildRustPackage {
        pname = cargoToml.package.name;
        version = cargoToml.package.version;
        src = self;
        cargoLock.lockFile = ./Cargo.lock;

        # The lifecycle tests spawn shell scripts they just wrote; run them
        # serially so a sibling test's fork can't hold a write fd on the
        # script being exec'd (ETXTBSY).
        dontUseCargoParallelTests = true;

        meta = {
          description = "Watches for file changes, rebuilds, and only restarts the server when the build succeeds";
          mainProgram = "cargo-serve";
          license = pkgs.lib.licenses.mit;
          platforms = pkgs.lib.platforms.unix;
        };
      };
  in
    {
      overlays.default = final: _prev: {
        cargo-serve = mkCargoServe final;
      };
    }
    // flake-utils.lib.eachDefaultSystem (
      system: let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [rust-overlay.overlays.default];
        };
        rust = pkgs.rust-bin.stable.latest.default.override {
          extensions = ["rust-src" "rust-analyzer"];
        };
      in {
        packages = rec {
          cargo-serve = mkCargoServe pkgs;
          default = cargo-serve;
        };

        apps = rec {
          cargo-serve = {
            type = "app";
            program = "${self.packages.${system}.cargo-serve}/bin/cargo-serve";
          };
          default = cargo-serve;
        };

        devShells.default = pkgs.mkShell {
          buildInputs = [rust];
        };

        formatter = pkgs.alejandra;
      }
    );
}

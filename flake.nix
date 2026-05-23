{
  description = "perch — real-time terminal dashboard for git changes";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    crane.url = "github:ipetkov/crane";
  };

  outputs = { self, nixpkgs, flake-utils, rust-overlay, crane }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ (import rust-overlay) ];
        };

        rustToolchain = (pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml).override {
          extensions = [ "rust-src" "rust-analyzer" ];
        };

        craneLib = (crane.mkLib pkgs).overrideToolchain rustToolchain;

        # Filter source to only include Cargo files and src/
        src = craneLib.cleanCargoSource ./.;

        commonArgs = {
          inherit src;
          strictDeps = true;

          buildInputs = pkgs.lib.optionals pkgs.stdenv.hostPlatform.isDarwin [
            pkgs.libiconv
          ];

          nativeBuildInputs = [
            pkgs.pkg-config
          ];
        };

        # Build only dependencies (cached layer)
        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        # Build the full package
        perch = craneLib.buildPackage (commonArgs // {
          inherit cargoArtifacts;
        });
      in
      {
        checks = {
          inherit perch;

          perch-clippy = craneLib.cargoClippy (commonArgs // {
            inherit cargoArtifacts;
            cargoClippyExtraArgs = "--all-targets -- --deny warnings";
          });

          perch-fmt = craneLib.cargoFmt {
            inherit src;
          };
        };

        packages = {
          default = perch;
          inherit perch;
        };

        devShells.default = craneLib.devShell {
          checks = self.checks.${system};

          packages = with pkgs; [
            cargo-watch
            cargo-edit
            cargo-outdated
            git
            pre-commit
          ];
        };
      }
    );
}

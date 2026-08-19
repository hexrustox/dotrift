{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs?ref=nixos-26.05";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-parts.url = "github:hercules-ci/flake-parts";
    nix-capsule.url = "gitlab:codnixus/nix-capsule?ref=v0.8.0";
  };

  outputs =
    { flake-parts, ... }@inputs:
    flake-parts.lib.mkFlake { inherit inputs; } {
      perSystem =
        {
          system,
          ...
        }:
        let
          pkgs = import inputs.nixpkgs {
            inherit system;
            overlays = [
              inputs.rust-overlay.overlays.default
              inputs.nix-capsule.overlays.default
            ];
          };
          capsule-lib = inputs.nix-capsule.lib { inherit pkgs; };
        in
        {
          apps.default = {
            type = "app";
            program = "${pkgs.ncap}/bin/ncap-direnv";
          };

          devShells = {
            default = capsule-lib.mkShell {
              image = "alpine:latest";
              devShell = "container";
              socketPath = "/tmp/dotrift/ncap-socket";
              containerName = "dotrift";
              extraOptions = [
                "-e"
                "HOME"
                "-e"
                "CARGO_HOME"
                "-v"
                "$CARGO_HOME:$CARGO_HOME"
                "-v"
                "$HOME/.cache/pnpm:/root/.cache/pnpm"
              ];
              wrappers = [
                "cargo"
                "codebook-lsp"
                "rust-analyzer"
                "taplo"
              ];
              preShellHook = ''
                export CARGO_HOME=''${CARGO_HOME:-$HOME/.cargo}
              '';
            };

            container =
              let
                rust = (
                  pkgs.rust-bin.stable."1.95.0".default.override {
                    extensions = [
                      "rust-src"
                      "rust-analyzer"
                      "llvm-tools-preview"
                    ];
                  }
                );
              in
              pkgs.mkShellNoCC {
                packages = with pkgs; [
                  cargo-deny
                  cargo-edit
                  cargo-insta
                  cargo-machete
                  cargo-llvm-cov
                  clang
                  codebook
                  mold
                  rust
                  taplo

                  less

                  skills
                  git
                ];
              };
          };
        };

      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
    };
}


{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs?ref=nixos-25.11";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-parts.url = "github:hercules-ci/flake-parts";
    nix-capsule.url = "gitlab:codnixus/nix-capsule?ref=v0.3.0";
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
          devShells = {
            default = capsule-lib.mkShell {
              image = "ubuntu:latest";
              devShell = "container";
              socketPath = "/tmp/dotrift/ncap-socket";
              containerName = "dotrift";
              options = [
                "-e HOME"
                "-e NIX_PATH"
                "-e HOME"
                "-v \"$HOME/.cargo\":\"$HOME/.cargo\""
              ];
              wrappers = [
                "cargo"
                "codebook-lsp"
                "rust-analyzer"
                "nixd"
                "taplo"
              ];
            };

            container =
              let
                rust = (
                  pkgs.rust-bin.stable."1.95.0".default.override {
                    extensions = [
                      "rust-src"
                      "rust-analyzer"
                      # "llvm-tools-preview"
                    ];
                  }
                );
              in
              pkgs.mkShellNoCC {
                packages = with pkgs; [
                  cargo-deny
                  cargo-edit
                  cargo-machete
                  clang
                  codebook
                  mold
                  nixd
                  nixfmt
                  rust
                  taplo

                  nano
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


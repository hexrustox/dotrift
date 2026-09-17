{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs?ref=nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-parts.url = "github:hercules-ci/flake-parts";
    nix-capsule.url = "github:hexrustox/nix-capsule?ref=v0.10.3";
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
              image = "alpine:latest";
              extraOptions = [
                "-e"
                "CARGO_HOME"
                "-v"
                "$CARGO_HOME:$CARGO_HOME"
              ];
              wrappers = [
                "cargo"
                "rust-analyzer"
                "taplo"
                "typos"
              ];
              preShellHook = ''
                export CARGO_HOME=''${CARGO_HOME:-$HOME/.cargo}
                mkdir -p "$CARGO_HOME"
              '';
            };

            container = pkgs.mkShellNoCC {
              packages = with pkgs; [
                (rust-bin.stable."1.95.0".default.override {
                  extensions = [
                    "rust-src"
                    "rust-analyzer"
                    "llvm-tools-preview"
                  ];
                })
                cargo-deny
                cargo-edit
                cargo-machete
                cargo-llvm-cov
                clang
                mold

                taplo

                typos

                skills
                git
              ];
            };
          };
        };

      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
    };
}


{
  description = "Invariant - a deterministic execution engine";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, fenix, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        src = pkgs.lib.cleanSource ./.;
        toolchain = fenix.packages.${system}.stable.withComponents [
          "cargo"
          "clippy"
          "rustc"
          "rustfmt"
          "rust-src"
          "rust-analyzer"
        ];
        wasmTarget = fenix.packages.${system}.targets.wasm32-wasip2.stable.rust-std;
        rustPlatform = pkgs.makeRustPlatform {
          cargo = toolchain;
          rustc = toolchain;
        };
        package = rustPlatform.buildRustPackage {
          pname = "invariant";
          version = "0.1.0";
          inherit src;
          cargoLock.lockFile = ./Cargo.lock;
        };
      in
      {
        formatter = pkgs.nixfmt-rfc-style;

        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            toolchain
            wasmTarget
            pkg-config
            openssl
            # WASM tools
            wasm-tools
            wasmtime
            # Go tools for building Go components
            go
            tinygo
            # Node.js for TypeScript/JavaScript components
            nodejs
            # Quint verification tool
            quint
          ];

          RUST_SRC_PATH = "${toolchain}/lib/rustlib/src/rust/library";
        };

        packages.default = package;

        checks = {
          default = package;
        };
      });
}

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
          "llvm-tools"
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
        cargoCrap = rustPlatform.buildRustPackage rec {
          pname = "cargo-crap";
          version = "0.2.0";

          src = pkgs.fetchCrate {
            inherit pname version;
            hash = "sha256-WH8bexRxJTs4ifpC8NxORjA939H+2/uMzOcR7TR8ggk=";
          };

          # Vendor deps from the lock file shipped inside the published crate.
          # This routes downloads through Nix's native `fetchurl` (the same path
          # the `invariant` package above uses), instead of buildRustPackage's
          # `cargoHash` / fetchCargoVendor path. The latter downloads with a
          # Python `requests` client whose `python-requests/*` User-Agent is
          # now rejected by crates.io's WAF with HTTP 403, which was failing
          # the change-risk gate in CI even when project code was fine.
          cargoLock.lockFile = "${src}/Cargo.lock";

          # Upstream CLI tests assume runtime commands that are not available in
          # the Nix sandbox. The packaged binary is validated by this repo's
          # change-risk check.
          doCheck = false;
        };
        # Kani Rust verifier CLI (`cargo kani` / `kani`).
        #
        # Kani is not a Cargo dependency of this workspace: the
        # `kani::any()` / `#[kani::proof]` API is injected by the Kani
        # toolchain at verification time and our usage is gated behind
        # `#[cfg(kani)]`, so normal `cargo build`/`cargo test` never see it.
        # What the dev shell needs is the `cargo kani` command itself.
        #
        # nixpkgs has no `kani` package, so we build the thin
        # `kani-verifier` installer crate here. That gives us the
        # `cargo-kani`/`kani` proxy binaries on PATH.
        #
        # As with `cargo-crap` above, deps are vendored from the lock file
        # shipped inside the published crate (`cargoLock.lockFile`) instead
        # of `cargoHash`, to avoid crates.io's HTTP 403 WAF rejection of
        # buildRustPackage's python-requests fetch path.
        kaniVerifier = rustPlatform.buildRustPackage (finalAttrs: {
          pname = "kani-verifier";
          version = "0.67.0";

          src = pkgs.fetchCrate {
            inherit (finalAttrs) pname version;
            hash = "sha256-m0khwmHJAiEtICN/f2IE70A2/0JNKwaL3so429YtdOY=";
          };

          cargoLock.lockFile = "${finalAttrs.src}/Cargo.lock";

          doCheck = false;
        });

        # ---------------------------------------------------------------
        # Hermetic Kani bundle (replaces the runtime `cargo kani setup`).
        #
        # The `kani-verifier` proxy binaries above only know how to *find
        # and exec* the real `kani-driver`; on first use they normally run
        # `cargo kani setup`, which (a) `curl`s the platform release bundle
        # from GitHub into `~/.kani/kani-<ver>/`, and (b) `rustup`-installs
        # the exact nightly that bundle was built against and symlinks it to
        # `~/.kani/kani-<ver>/toolchain`. Both steps hit the network and are
        # the only non-Nix dependency left in the dev shell.
        #
        # We reproduce that already-set-up layout entirely through Nix:
        #
        #   * `kaniBundleSrc` — the official prebuilt release tarball,
        #     fetched by `fetchurl` with a PINNED sha256 (a fixed-output
        #     derivation; prebuilt-binary toolchains are fetched this way
        #     throughout nixpkgs). The bundle ships its own `cbmc`,
        #     `goto-cc`, `goto-instrument`, `goto-analyzer`, `kissat` and
        #     `kani-compiler` under `bin/`, plus the precompiled kani libs.
        #
        #   * `kaniRustToolchain` — the matching nightly from fenix. The
        #     bundle does NOT ship a toolchain; it only records the version
        #     it needs (`rust-toolchain-version` = nightly-2025-11-21,
        #     `rustc-version` = 1.93.0-nightly (53732d5e0 2025-11-20)).
        #     fenix builds that exact nightly hermetically, replacing the
        #     `rustup toolchain install` step. The pinned manifest hash is
        #     verified by fenix's own fixed-output derivation.
        #
        #   * `kaniHome` — assembles `$out/kani-<ver>/` to look exactly like
        #     a completed `cargo kani setup`: the unpacked bundle plus a
        #     `toolchain` symlink into the fenix nightly. The driver finds
        #     everything relative to its own location
        #     (`base_folder = bin/..`) and, in Release mode, invokes
        #     `kani-<ver>/toolchain/bin/cargo` directly, so a read-only
        #     store path is sufficient (the driver writes only to the
        #     project's `target/`).
        #
        # The dev shell then sets `KANI_HOME` to `$out`; the proxy reads it
        # (`${KANI_HOME}/kani-<ver>`), sees the layout already present
        # (`appears_setup()`), and execs the bundled driver with NO network
        # access and NO `~/.kani` download.
        #
        # On Linux the prebuilt ELF binaries need their interpreter and
        # rpath patched for the Nix store (the same fixup `cargo kani setup`
        # does via its NixOS `patchelf` hack); on macOS the Mach-O binaries
        # run as-is.
        kaniVersion = "0.67.0";
        kaniTarget = {
          aarch64-darwin = "aarch64-apple-darwin";
          x86_64-darwin = "x86_64-apple-darwin";
          x86_64-linux = "x86_64-unknown-linux-gnu";
          aarch64-linux = "aarch64-unknown-linux-gnu";
        }.${system};
        kaniBundleHash = {
          aarch64-darwin = "sha256-f9C3ETCqN70eNG66Z1qtCem2bks4P3B/xlczDOsp7uw=";
          x86_64-darwin = null; # not separately prefetched; add hash if needed
          x86_64-linux = "sha256-O196/TtRYD7nINt7wbxP5GtaT1022q2ZOcS0xli1GsA=";
          aarch64-linux = "sha256-l0Eo9E3UNhigbSHl/m2f9nGI3lmG/gvFe1NLDkY577k=";
        }.${system};
        kaniBundleSrc = pkgs.fetchurl {
          url = "https://github.com/model-checking/kani/releases/download/kani-${kaniVersion}/kani-${kaniVersion}-${kaniTarget}.tar.gz";
          hash = kaniBundleHash;
        };
        # The exact nightly the 0.67.0 bundle was built against. Pinned by
        # date; fenix verifies the channel manifest with `sha256` below.
        kaniRustToolchain =
          (fenix.packages.${system}.toolchainOf {
            channel = "nightly";
            date = "2025-11-21";
            sha256 = "sha256-P39FCgpfDT04989+ZTNEdM/k/AE869JKSB4qjatYTSs=";
          }).toolchain;
        kaniHome = pkgs.runCommand "kani-home-${kaniVersion}"
          {
            nativeBuildInputs = pkgs.lib.optionals pkgs.stdenv.isLinux [ pkgs.patchelf ];
          }
          ''
            dir="$out/kani-${kaniVersion}"
            mkdir -p "$dir"
            tar --strip-components=1 -xzf ${kaniBundleSrc} -C "$dir"

            # Recreate the `cargo kani setup` toolchain symlink, but point
            # it at the hermetic fenix nightly instead of a rustup install.
            ln -s ${kaniRustToolchain} "$dir/toolchain"

            ${pkgs.lib.optionalString pkgs.stdenv.isLinux ''
              # Linux release binaries are prebuilt against a standard FHS
              # loader and a system libstdc++; patch them for the Nix store.
              interp="$(cat ${pkgs.stdenv.cc}/nix-support/dynamic-linker)"
              rpath="${pkgs.stdenv.cc.cc.lib}/lib"
              for f in kani-compiler kani-driver; do
                patchelf --set-interpreter "$interp" "$dir/bin/$f" || true
              done
              for f in cbmc goto-analyzer goto-cc goto-instrument kissat; do
                patchelf --set-interpreter "$interp" "$dir/bin/$f" || true
                patchelf --set-rpath "$rpath" "$dir/bin/$f" || true
              done
            ''}
          '';
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
            # Kani Rust verifier (`cargo kani`). The `cargo-kani`/`kani`
            # proxy binaries come from this package; the Kani compiler
            # bundle + matching nightly toolchain are provided hermetically
            # by `kaniHome` (see above) and wired up via `KANI_HOME` below,
            # so there is NO runtime `cargo kani setup` download into
            # `~/.kani`. The bundle ships its OWN `cbmc`, `goto-cc`,
            # `goto-instrument`, `goto-analyzer`, and `kissat` binaries in
            # `kani-<ver>/bin` (which the driver puts on PATH), so the dev
            # shell does NOT need nixpkgs `cbmc`/`kissat`.
            kaniVerifier
            # Testing tools
            cargo-nextest
            cargo-llvm-cov
            cargo-insta
            cargoCrap
            just
          ];

          RUST_SRC_PATH = "${toolchain}/lib/rustlib/src/rust/library";

          # Point Kani's proxy at the hermetic, pre-staged bundle so it
          # never runs `cargo kani setup`. The proxy resolves the install
          # as `${KANI_HOME}/kani-<ver>` and execs the bundled driver.
          KANI_HOME = "${kaniHome}";
        };

        packages.default = package;

        checks = {
          default = package;
        };
      });
}

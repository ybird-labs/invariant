set dotenv-load := true

# Run a command inside the Nix development shell.
nix *args:
    nix develop --accept-flake-config -c {{args}}

# Check Rust formatting.
_fmt-check:
    cargo fmt --all -- --check

fmt-check:
    nix develop --accept-flake-config -c just _fmt-check

# Format Rust code.
_fmt:
    cargo fmt --all

fmt:
    nix develop --accept-flake-config -c just _fmt

# Run clippy with warnings denied.
_clippy:
    cargo clippy --workspace --all-targets -- -D warnings

clippy:
    nix develop --accept-flake-config -c just _clippy

# Run the workspace test suite with nextest.
_test:
    cargo nextest run --workspace --all-targets --no-fail-fast --profile ci

test:
    nix develop --accept-flake-config -c just _test

# Run Rust doc tests.
_doc-test:
    cargo test --workspace --doc

doc-test:
    nix develop --accept-flake-config -c just _doc-test

# Run the existing Rust quality gate: fmt, clippy, tests, and doc tests.
_rust-quality:
    ./scripts/ci/rust-quality.sh

rust-quality:
    nix develop --accept-flake-config -c just _rust-quality

# Run Nix flake checks.
_flake-check:
    nix flake check --accept-flake-config --print-build-logs

flake-check:
    nix develop --accept-flake-config -c just _flake-check

# Generate LCOV coverage for cargo-crap.
_coverage:
    cargo llvm-cov --workspace --all-targets --lcov --output-path lcov.info

coverage:
    nix develop --accept-flake-config -c just _coverage

# Run cargo-crap against an existing LCOV report.
_crap lcov="lcov.info" threshold="30":
    cargo crap --workspace --lcov {{lcov}} --format github --fail-above --threshold {{threshold}}

crap lcov="lcov.info" threshold="30":
    nix develop --accept-flake-config -c just _crap {{lcov}} {{threshold}}

# Generate coverage and run the cargo-crap change-risk gate.
_change-risk threshold="30": _coverage
    cargo crap --workspace --lcov lcov.info --format github --fail-above --threshold {{threshold}}

change-risk threshold="30":
    nix develop --accept-flake-config -c just _change-risk {{threshold}}

# Run the same local gates as CI, excluding the cross-platform flake matrix.
_ci: _rust-quality _change-risk

ci:
    nix develop --accept-flake-config -c just _ci

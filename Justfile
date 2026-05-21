set dotenv-load := true

# Run a command inside the Nix development shell.
nix *args:
    nix develop --accept-flake-config -c {{args}}

# Check Rust formatting.
fmt-check:
    nix develop --accept-flake-config -c cargo fmt --all -- --check

# Format Rust code.
fmt:
    nix develop --accept-flake-config -c cargo fmt --all

# Run clippy with warnings denied.
clippy:
    nix develop --accept-flake-config -c cargo clippy --workspace --all-targets -- -D warnings

# Run the workspace test suite with nextest.
test:
    nix develop --accept-flake-config -c cargo nextest run --workspace --all-targets --no-fail-fast --profile ci

# Run Rust doc tests.
doc-test:
    nix develop --accept-flake-config -c cargo test --workspace --doc

# Run the existing Rust quality gate: fmt, clippy, tests, and doc tests.
rust-quality:
    nix develop --accept-flake-config -c ./scripts/ci/rust-quality.sh

# Generate LCOV coverage for cargo-crap.
coverage:
    nix develop --accept-flake-config -c cargo llvm-cov --workspace --all-targets --lcov --output-path lcov.info

# Install cargo-crap into the local Cargo bin directory if it is missing.
install-cargo-crap:
    nix develop --accept-flake-config -c bash -lc 'command -v cargo-crap >/dev/null || cargo install cargo-crap --locked'

# Run cargo-crap against an existing LCOV report.
crap lcov="lcov.info" threshold="30":
    nix develop --accept-flake-config -c cargo crap --lcov {{lcov}} --format github --fail-above --threshold {{threshold}}

# Generate coverage and run the cargo-crap change-risk gate.
change-risk threshold="30": install-cargo-crap coverage
    nix develop --accept-flake-config -c cargo crap --lcov lcov.info --format github --fail-above --threshold {{threshold}}

# Run the same local gates as CI, excluding the cross-platform flake matrix.
ci: rust-quality change-risk

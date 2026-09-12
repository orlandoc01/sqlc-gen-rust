_default:
  just --list 

set dotenv-filename := ".dev.env"
set dotenv-load

alias f:= format
alias l:= lint
alias lf:= lint-fix

setup-tools:
    rustup target add wasm32-wasip1

# format
format:
    cargo fmt --all

# format in CI
format-ci:
    RUSTFLAGS="--deny warnings" cargo fmt --all --check

# Show lint error
lint:
    cargo clippy --workspace --all-targets --all-features 

# Fix clippy error
lint-fix:
    cargo clippy --fix --workspace --all-targets --all-features --allow-dirty --allow-staged

# lint in CI
lint-ci:
    RUSTFLAGS="--deny warnings" cargo clippy --workspace --all-targets --all-features

# Run required local checks
check-local:
    just format-ci
    just lint-ci
    just generate
    git diff --exit-code
    cargo test -p sqlc-gen-rust -p authors-sqlx-sqlite -p dynamic-filter-sqlx-sqlite -p sqlc-slice-sqlx-sqlite -p type-mapping-sqlx-sqlite -p embed-sqlx-sqlite
    cargo clippy -p authors-sqlx-postgres -p authors-sqlx-mysql -p authors-sqlx-sqlite -p dynamic-filter-sqlx-mysql -p dynamic-filter-sqlx-postgres -p dynamic-filter-sqlx-sqlite -p embed-sqlx-sqlite -p sqlc-slice-sqlx-postgres -p sqlc-slice-sqlx-mysql -p sqlc-slice-sqlx-sqlite -p type-mapping-sqlx-postgres -p type-mapping-sqlx-mysql -p type-mapping-sqlx-sqlite -p e-commerce --all-targets -- -D warnings

# Run tests
test:
    cargo test --workspace

# rebuild plugin and generate sqlc
generate:
    #!/usr/bin/env bash
    set -euxo pipefail

    cargo build --target wasm32-wasip1

    WASM_SHA256=$(sha256sum target/wasm32-wasip1/debug/sqlc-gen-rust.wasm | awk '{print $1}');
    sed "s/\$WASM_SHA256/${WASM_SHA256}/g" sqlc.yaml > _sqlc_dev.yaml
    sqlc generate -f _sqlc_dev.yaml

    rm _sqlc_dev.yaml
    just f

# build plugin and generate sqlc
generate-release:
    #!/usr/bin/env bash
    set -euxo pipefail

    cargo build --target wasm32-wasip1 --release --locked

    WASM_SHA256=$(sha256sum target/wasm32-wasip1/release/sqlc-gen-rust.wasm | awk '{print $1}');
    sed "s/\$WASM_SHA256/${WASM_SHA256}/g" sqlc.yaml | sed "s/debug/release/g" > _sqlc_dev.yaml
    sqlc generate -f _sqlc_dev.yaml

    rm _sqlc_dev.yaml
    cargo fmt --all

build-release:
    #!/usr/bin/env bash
    set -euxo pipefail

    cargo build --target wasm32-wasip1 --release --locked
    WASM_SHA256=$(sha256sum target/wasm32-wasip1/release/sqlc-gen-rust.wasm | awk '{print $1}');
    echo ${WASM_SHA256}

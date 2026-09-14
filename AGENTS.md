# AGENTS.md

## Scope

This is a public fork of `tunamaguro/sqlc-gen-rust`, a sqlc WASM plugin that generates Rust. It is published from `github.com/orlandoc01/sqlc-gen-rust`

- `main` carries `sqlc.embed` support, params-struct output, and `-- :if` dynamic filters.
- New behavior must be behind an opt-in config option. Dynamic filters are opted into per query by the `-- :if` annotation, not by a separate option.
- Never hand-edit committed generated output. Run `just generate`, including for files such as `examples/*/src/queries.rs`.

## Layout

- `src/lib.rs`: plugin entry point, `Config` parsing and validation, and override types.
- `src/query.rs`: query, parameter, and row models; annotations; and static-slice handling.
- `src/dynfilter.rs`: parses `-- :if @param` annotations and static `sqlc.slice()` markers into a bind plan.
- `src/db_crates/{mod,sqlx,rusqlite,postgres}.rs`: backend dispatch, SQLx/SQLite/PostgreSQL/deadpool-postgres type mapping, and backend setup and row decoding.
- `src/db_crates/params_common.rs`: shared params-struct definitions, identifier generation, dynamic-filter setup, and query index.
- `src/db_crates/{sqlx_params,rusqlite_params,postgres_params}.rs`: SQLx, rusqlite, and postgres/tokio-postgres/deadpool-postgres params-struct query generators.
- `src/db_crates/{rusqlite_tests,postgres_tests}.rs`: rusqlite and postgres/tokio-postgres/deadpool-postgres generator token tests.
- `src/db_crates/postgres_types.rs`: PostgreSQL type mappings shared by the SQLx, postgres, tokio-postgres, and deadpool-postgres backends.
- `src/db_crates/dynfilter_runtime.rs`: the `pub mod dynfilter` runtime that is inlined into generated code when a query needs it. It is also compiled into the plugin's own tests.
- `src/path_map.rs`: SQL-to-Rust path mapping.
- `src/protos/codegen.proto`: sqlc plugin protocol. `build.rs` compiles it with `prost-build`, which requires `protoc`.
- Root `sqlc.yaml`: drives regeneration for every `examples/*` package.
- `examples/dynamic-filter/*`: one crate per supported engine exercising `-- :if`.
- `examples/test-utils`: SQLx, postgres, tokio-postgres, deadpool-postgres, MySQL, and SQLite test contexts; PostgreSQL/MySQL read their database URLs from the environment.
- `rust-toolchain.toml`: pins Rust 1.89.0 and the `wasm32-wasip1` target.
- `.devcontainer/` + `Dockerfile` + `compose.yaml`: upstream's VS Code container with `postgres` and `mysql` services. `.dev.env` uses the compose hostnames, which only resolve inside that container.

## Tools

Building needs `protoc`, `just`, and `sqlc` 1.31.1 on `PATH`, plus the toolchain from `rust-toolchain.toml`. CI and the devcontainer pin the same sqlc version; keep the three in sync when bumping.

## Gates

Before a commit is done, run the one-command gate:

```sh
just check-local
```

It runs `just format-ci`, `just lint-ci`, `just generate && git diff --exit-code`, then `cargo test` and `cargo clippy` over the plugin and the SQLite-backed example crates listed in the `check-local` recipe in `Justfile`. That recipe is the source of truth for which crates are covered locally.

PostgreSQL/MySQL example tests need running databases. From the host, `.dev.env` does not work; start the databases with published ports and export the URLs yourself, for example:

```sh
docker run -d --rm --name sqlcrust-pg -p 127.0.0.1:5433:5432 -e POSTGRES_USER=root -e POSTGRES_PASSWORD=password -e POSTGRES_DB=app postgres:17.0-bookworm
docker run -d --rm --name sqlcrust-mysql -p 127.0.0.1:3307:3306 -e MYSQL_ROOT_PASSWORD=password -e MYSQL_DATABASE=app mysql:9.4
export POSTGRES_DATABASE_URL=postgres://root:password@127.0.0.1:5433/app
export MYSQL_DATABASE_URL=mysql://root:password@127.0.0.1:3307/app
cargo test --workspace
```

## CI

`.github/workflows/pull_request.yaml` runs on PRs and pushes to `main`: format, clippy, then a test job with Postgres and MySQL service containers that runs `just generate-release`, fails on any diff in generated output, and runs `just test` (the whole workspace). `security.yaml` runs `cargo audit` (exceptions in `.cargo/audit.toml`) and a full-history gitleaks scan on PRs, pushes to `main`, and weekly. `dependabot_automerge.yaml` auto-merges grouped minor/patch GitHub Actions updates.

## Releases

`.github/workflows/release.yaml` runs on a `v*` tag push. A verify job checks that the tag matches `Cargo.toml`'s version and that a successful `Rust CI` run exists for the tagged commit; the release job then builds the wasm with `--locked`, attaches `sqlc-gen-rust.wasm` and its `.sha256` to a GitHub release, and generates notes.

The wasm is not byte-reproducible across machines: a local `just build-release` hashes differently from CI's build of the same commit. The README sha256 must therefore come from the CI-built asset, never from a local build. To cut a release:

1. Bump `version` in `Cargo.toml`, run `just generate` (generated file headers embed the version), commit.
2. Tag that commit `vX.Y.Z` on the public `main` and push the tag; wait for the release workflow.
3. Copy the sha256 from the release's `sqlc-gen-rust.wasm.sha256` asset into the README install snippet along with the new tag, commit, push. That commit is docs-only and needs no new tag.

## Machine Traps

- Do not write scratch files outside the repository except under `/tmp`.
- Do not hand-edit generated output, including `examples/*/src/queries.rs`.
- Bumping `version` in `Cargo.toml` changes every generated `queries.rs` header; regenerate in the same commit or the CI drift check fails.

## Style

Match upstream: follow `rustfmt.toml` and keep Clippy clean with `--deny warnings`. Do not write doc comments that restate code; reserve comments for non-obvious constraints.

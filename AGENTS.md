# AGENTS.md

## Scope

This is a private fork of `tunamaguro/sqlc-gen-rust`, a sqlc WASM plugin that generates Rust.

- Keep `main` limited to upstream parity plus `sqlc.embed` support, so it remains suitable for a future upstream PR.
- Put opinionated features, including params-struct output and `-- :if` dynamic filters, on `dynfilter`.
- New behavior must be behind an opt-in config option; upstream behavior remains the default.
- Never hand-edit committed generated output. Run `just generate`, including for files such as `examples/*/src/queries.rs`.

## Layout

- `src/lib.rs`: plugin entry point, `Config` parsing, and override types.
- `src/query.rs`: query, parameter, and row models; annotations; and `SLICE` expansion.
- `src/db_crates/{mod,sqlx,postgres,rusqlite}.rs`: per-crate code generation. `sqlx.rs` covers sqlx-postgres, sqlx-mysql, and sqlx-sqlite.
- `src/path_map.rs`: SQL-to-Rust path mapping.
- `src/protos/codegen.proto`: sqlc plugin protocol. `build.rs` compiles it with `prost-build`, which requires `protoc`.
- Root `sqlc.yaml`: drives regeneration for every `examples/*` package.
- `examples/test-utils`: PostgreSQL/MySQL test contexts reading `POSTGRES_DATABASE_URL` and `MYSQL_DATABASE_URL` from `.dev.env`; they need Docker Compose.
- `rust-toolchain.toml`: pins Rust 1.89.0 and the `wasm32-wasip1` target.

## Tools

`mise` globally installs `protoc` 36.1, `just` 1.58.0, and `sqlc` 1.31.1. If one is not on `PATH`, invoke it with `mise exec -- <cmd>`.

## Gates

Before a commit is done, run the one-command gate:

```sh
just check-local
```

It runs `just format-ci`, `just lint-ci`, `just generate && git diff --exit-code`, and:

```sh
cargo test -p sqlc-gen-rust -p authors-sqlx-sqlite -p sqlc-slice-sqlx-sqlite -p type-mapping-sqlx-sqlite
```

The current baseline is 9 plugin tests and 5 SQLite example tests passing. PostgreSQL/MySQL example tests are optional locally; run `docker compose up -d postgres mysql` first. This fork has no CI yet.

## Machine Traps

- Shell output may be rewritten by an `rtk` hook. When parsing command output, use `rtk proxy <cmd>` or absolute binary paths.
- Do not write scratch files outside the repository except under `/tmp`.
- Do not hand-edit generated output, including `examples/*/src/queries.rs`.

## Style

Match upstream: follow `rustfmt.toml` and keep Clippy clean with `--deny warnings`. Do not write doc comments that restate code; reserve comments for non-obvious constraints.

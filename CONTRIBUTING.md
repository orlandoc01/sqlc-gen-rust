# Contributing to sqlc-gen-rust

sqlc-gen-rust is a fork of [tunamaguro/sqlc-gen-rust](https://github.com/tunamaguro/sqlc-gen-rust) maintained by one person. Contributions are welcome, and review time is best-effort.

## Before You Start

- Use issues for bug reports, feature requests, and design discussion.
- Open an issue before starting a large change, a new backend, or a broad refactor.
- Keep pull requests focused. Small, reviewable changes are far more likely to be merged.
- New generator behavior must be opt-in through a config option. Dynamic filters are opted into per query by the `-- :if` annotation rather than by a separate option.
- Do not include real database contents, credentials, connection strings, or private schema details in issues, logs, fixtures, or tests. Sanitize schemas and queries before sharing them.

Unsolicited large pull requests may not be reviewed or merged, even when the code works, if they do not fit the maintainer's current goals or capacity.

## Development Setup

Read `AGENTS.md` first. It describes the repository layout, the tools, and the local gate.

You need:

- The Rust toolchain pinned in `rust-toolchain.toml` (rustup installs it, including the `wasm32-wasip1` target).
- `protoc`, `just`, and `sqlc`. The versions the maintainer uses are listed in `AGENTS.md`; CI pins the same sqlc version.
- Docker, only for the PostgreSQL and MySQL example tests.

`just generate` builds the plugin, regenerates every `examples/*` crate from the root `sqlc.yaml`, and formats the result.

## Testing and Quality Gates

Run the local gate before opening a pull request:

```sh
just check-local
```

It checks formatting, runs Clippy with warnings denied, regenerates the examples and fails on any diff, then runs the plugin tests and the SQLite-backed example tests.

The PostgreSQL and MySQL example tests need running databases. `AGENTS.md` shows how to start them with Docker and which environment variables to export; then run the whole workspace:

```sh
cargo test --workspace
```

If you cannot run a relevant check, say so in the pull request and explain why.

## Generated Output

Files such as `examples/*/src/queries.rs` are generated. Never hand-edit them. After changing the generator, run `just generate` and commit the refreshed output in the same change. CI regenerates everything and fails on any drift.

## Code Style

- Follow `rustfmt.toml` and keep Clippy clean under `--deny warnings`.
- Do not write comments that restate the code. Reserve comments for non-obvious constraints.
- Add or update tests when generator behavior changes. Generator token tests live next to each backend in `src/db_crates/`; behavior tests live in the example crates.
- Update the README when a configuration option or user-visible generated API changes.

## Pull Request Checklist

- The change is scoped to one concern.
- `just check-local` passes, and database-backed tests pass if the change touches PostgreSQL or MySQL code.
- Generated output is refreshed with `just generate`.
- Documentation is updated when options, generated APIs, or commands change.
- No credentials, private URLs, or real database contents are included.

## License

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed under the MIT and Apache-2.0 licenses as described in the [README](README.md#license), without any additional terms or conditions.

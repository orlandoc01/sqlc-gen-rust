# Security Policy

sqlc-gen-rust is a sqlc plugin that generates Rust database code. It runs inside sqlc's WASM sandbox at generation time and ships no runtime service, but the code it emits runs against your database. Please report security issues privately so users can update before details are public.

## Supported Versions

The plugin is pre-1.0 software. Security fixes are best-effort and target the latest release and the `main` branch. Older releases do not receive patches unless a release note says otherwise.

## Reporting a Vulnerability

Use [GitHub private vulnerability reporting](https://github.com/orlandoc01/sqlc-gen-rust/security/advisories/new) for this repository. If that is unavailable, email the maintainer at the address listed on their GitHub profile.

Do not open a public issue for vulnerabilities. Please include:

- A summary of the issue and its likely impact.
- The affected plugin version or the wasm checksum, and the sqlc version.
- The `db_crate` and backend crate versions involved.
- A minimal sanitized schema, query, and `sqlc.yaml` that reproduces the problem.
- The generated code or runtime behavior you observed, with credentials and database contents removed.

## Response Expectations

This is a solo-maintained project. The maintainer makes a best-effort attempt to acknowledge valid reports within 7 days; responses may be slower depending on availability.

Valid reports are handled by confirming the issue and affected versions, preparing a fix, publishing a release or patch guidance when practical, and crediting the reporter if requested.

## Scope

In scope:

- Generated SQL or bind plans that differ from the source query in a way that can change which rows are read or written, including placeholder renumbering and `-- :if` segment handling.
- Generated code that exposes data it should not, such as decoding the wrong column into a field.
- Dependency vulnerabilities with a concrete impact on the plugin wasm or on generated code.
- Release integrity: a published `sqlc-gen-rust.wasm` that does not match its checksum or the tagged source.

Out of scope unless there is a concrete exploit path:

- Bugs in SQL you wrote, or in sqlc itself. Report sqlc issues upstream.
- Dependency advisories that only affect the example crates or build-time tooling and do not reach the plugin wasm or generated code.
- Denial of service of the generator that requires a malicious schema or query you control.

## Safe Harbor

Good-faith research is welcome when it avoids data destruction, service disruption, and public disclosure before a fix is available. Do not test against databases you do not own or operate.

## No Bug Bounty

There is no paid bug bounty program. Reports are appreciated, but compensation is not offered.

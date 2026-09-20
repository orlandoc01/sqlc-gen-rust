# sqlc-gen-rust

[![Rust CI](https://github.com/orlandoc01/sqlc-gen-rust/actions/workflows/pull_request.yaml/badge.svg)](https://github.com/orlandoc01/sqlc-gen-rust/actions/workflows/pull_request.yaml)
[![Release](https://img.shields.io/github/v/release/orlandoc01/sqlc-gen-rust)](https://github.com/orlandoc01/sqlc-gen-rust/releases)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](LICENSE-MIT)

[sqlc](https://sqlc.dev/) plugin that turns SQL queries into typed Rust functions for SQLx, rusqlite, postgres, tokio-postgres, and deadpool-postgres.

- Write SQL and generate functions, parameter structs, and row types.
- Use [dynamic filters](#dynamic-filters) for optional predicates, joins, and sort presets.
- Opt into [statement caching](#dynfilters).

Forked from [tunamaguro/sqlc-gen-rust](https://github.com/tunamaguro/sqlc-gen-rust), with dynamic filters inspired by [vtuanjs/sqlc-gen-go](https://github.com/vtuanjs/sqlc-gen-go).

## Contents

- [Usage](#usage)
- [Supported crates](#supported-crates)
- [Example](#example)
- [Supported features](#supported-features)
- [Dynamic filters](#dynamic-filters)
  - [Annotation placement](#annotation-placement)
  - [Gated joins](#gated-joins)
  - [`OR` groups](#or-groups)
  - [Filtering for `NULL`](#filtering-for-null)
  - [Validation and limitations](#validation-and-limitations)
  - [Runtime behavior](#runtime-behavior)
- [Backend notes](#backend-notes)
  - [sqlx-postgres, sqlx-mysql, sqlx-sqlite](#sqlx-postgres-sqlx-mysql-sqlx-sqlite)
  - [rusqlite](#rusqlite)
  - [postgres](#postgres)
  - [tokio-postgres](#tokio-postgres)
  - [deadpool-postgres](#deadpool-postgres)
  - [PostgreSQL drivers](#postgresql-drivers)
- [Options](#options)
  - [`db_crate`](#db_crate)
  - [`dynfilters`](#dynfilters)
  - [`query_parameter_limit`](#query_parameter_limit)
  - [`overrides`](#overrides)
  - [`row_attributes` / `column_attributes`](#row_attributes--column_attributes)
  - [`enum_derives`](#enum_derives)
  - [`output`](#output)
- [Compatibility](#compatibility)
- [Credits](#credits)
- [License](#license)
- [Contribution](#contribution)

## Usage

The example below uses PostgreSQL and SQLx. Install [sqlc](https://docs.sqlc.dev/en/latest/overview/install.html) (tested with 1.31.1), then:

1. Create `schema.sql` and `queries.sql` using the [example](#example) below.
2. Save this configuration as `sqlc.yaml` at your crate root:

```yaml
version: "2"
plugins:
  - name: sqlc-gen-rust
    wasm:
      url: https://github.com/orlandoc01/sqlc-gen-rust/releases/download/v0.2.0/sqlc-gen-rust.wasm
      sha256: <sha256 from the v0.2.0 release asset sqlc-gen-rust.wasm.sha256>
sql:
  - schema: schema.sql
    queries: queries.sql
    engine: postgresql
    codegen:
      - plugin: sqlc-gen-rust
        out: src/
        options:
          db_crate: sqlx-postgres
          output: queries.rs
```

3. Add the dependencies for this example:

```sh
cargo add sqlx@0.8 --features runtime-tokio,postgres
cargo add tokio@1 --features macros,rt-multi-thread
```

4. Generate `src/queries.rs`:

```sh
sqlc generate
```

5. Apply `schema.sql` to your PostgreSQL database, set `DATABASE_URL`, and use the [generated API](#generated-api) from your application with `mod queries;`.

sqlc downloads the pinned plugin automatically, you don't need to build or add the plugin as a Rust dependency. Generation does not apply your schema to a database. Rerun `sqlc generate` whenever your SQL or configuration changes.

For another backend, change both `engine` and [`db_crate`](#db_crate), and use that backend's SQL syntax and Rust dependencies.

## Supported crates

| `db_crate` | sqlc `engine` | API |
| ---------- | ------------- | --- |
| [sqlx-postgres](https://docs.rs/sqlx/latest/sqlx/postgres/index.html) | `postgresql` | Async |
| [sqlx-mysql](https://docs.rs/sqlx/latest/sqlx/mysql/index.html) | `mysql` | Async |
| [sqlx-sqlite](https://docs.rs/sqlx/latest/sqlx/sqlite/index.html) | `sqlite` | Async |
| [rusqlite](https://docs.rs/rusqlite/latest/rusqlite/) | `sqlite` | Sync |
| [postgres](https://docs.rs/postgres/latest/postgres/) | `postgresql` | Sync |
| [tokio-postgres](https://docs.rs/tokio-postgres/latest/tokio_postgres/) | `postgresql` | Async |
| [deadpool-postgres](https://docs.rs/deadpool-postgres/latest/deadpool_postgres/) | `postgresql` | Async, pooled |

See [Backend notes](#backend-notes) for crate-specific behavior.

## Example

### Schema

```sql
-- schema.sql
CREATE TABLE authors (
    id   BIGSERIAL PRIMARY KEY,
    name text      NOT NULL,
    bio  text
);
```

### Query

```sql
-- queries.sql
-- name: GetAuthor :one
SELECT * FROM authors
WHERE id = $1 LIMIT 1;

-- name: ListAuthors :many
SELECT * FROM authors
ORDER BY name;

-- name: CreateAuthor :one
INSERT INTO authors (
          name, bio
) VALUES (
  $1, $2
)
RETURNING *;

-- name: DeleteAuthor :exec
DELETE FROM authors
WHERE id = $1;
```

### Generated API

Each query becomes a free function and a SQL constant. Queries that return rows also generate a row struct. By default, functions take:

- No parameters: just the database handle.
- One parameter: the database handle and the parameter value.
- Two or more parameters: the database handle and a public params struct.

Use [`query_parameter_limit`](#query_parameter_limit) to change this threshold. The database handle depends on the backend, see [Backend notes](#backend-notes).

With the SQLx PostgreSQL configuration above, put this in `src/main.rs`:

```rust
mod queries;

use queries::CreateAuthorParams;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pool = sqlx::PgPool::connect(&std::env::var("DATABASE_URL")?).await?;

    let author = queries::create_author(
        &pool,
        CreateAuthorParams {
            name: "John",
            bio: Some("Foo"),
        },
    )
    .await?;
    let fetched = queries::get_author(&pool, author.id).await?;
    assert_eq!(fetched.name, "John");

    let authors = queries::list_authors(&pool).await?;
    println!("{} authors", authors.len());

    queries::delete_author(&pool, author.id).await?;
    Ok(())
}
```

For `:one` queries, the generated `<fn>_opt` variant returns `Result<Option<Row>, Error>` when a missing row is expected: `queries::get_author_opt(&pool, id).await?`. See [Backend notes](#backend-notes) for runnable examples for each crate.

## Supported features

### Query annotations

| Backend | `:exec` | `:execrows` | `:execresult` | `:execlastid` | `:many` | `:one` |
| ------- | ------- | ----------- | ------------- | ------------- | ------- | ------ |
| sqlx-postgres | ✅ | ✅ | ✅ | ❌ | ✅ | ✅ |
| sqlx-mysql | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| sqlx-sqlite | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| rusqlite | ✅ | ✅ | ❌ | ✅ | ✅ | ✅ |
| postgres | ✅ | ✅ | ✅ | ❌ | ✅ | ✅ |
| tokio-postgres / deadpool-postgres | ✅ | ✅ | ✅ | ❌ | ✅ | ✅ |

`:copyfrom` and `:batch*` queries are not supported.

### Macros

| Macro        | Status |
| ------------ | ------ |
| `sqlc.arg`   | ✅      |
| `sqlc.embed` | ✅      |
| `sqlc.narg`  | ✅      |
| `sqlc.slice` | ✅      |

### `sqlc.embed`

`sqlc.embed(table)` emits a nested model struct for the table and decodes its columns by their SELECT position. See the [embed examples](./examples/embed/) for joined rows.

### `sqlc.slice`

PostgreSQL binds slices as arrays. MySQL, SQLx SQLite, and rusqlite expand slice markers through the generated dynamic bind plan.

## Dynamic filters

Use SQL comments to make filters, joins, and ordering selectable at runtime. Add annotations to your query and run `sqlc generate`, no configuration option is needed. The generated code handles separators and bind parameters when parts of the query are omitted.

Annotations apply to complete `WHERE` conditions joined by `AND`, operands in parenthesized `OR` groups, `ORDER BY` terms, and supported `JOIN ... ON ...` clauses.

| Annotation | Gates on | Generated field |
| ---------- | -------- | --------------- |
| `-- :if @param` | `Some(value)` keeps the SQL fragment and binds the value, `None` omits it | `param: Option<T>` |
| `-- :flag @name` | `true` keeps the SQL fragment, `false` omits it. The annotation declares the flag | `name: bool` |
| `-- :switch @field a b default=a` with `-- :case @a` / `-- :case @b` | One preset in a `WHERE` or `ORDER BY` clause | `field: <Query><Field>` enum with a default variant |

For example, this PostgreSQL query combines optional filters with a sort preset:

```sql
-- name: SearchUsers :many
SELECT * FROM users
WHERE
  email = @email -- :if @email
  AND phone = @phone -- :if @phone
  AND EXISTS ( -- :flag @has_orders
    SELECT 1 FROM orders WHERE orders.user_id = users.id
  )
ORDER BY -- :switch @sort id_asc id_desc shortest_email default=id_asc
  id ASC,                    -- :case @id_asc
  id DESC,                   -- :case @id_desc
  LENGTH(email) ASC, id DESC -- :case @shortest_email
LIMIT sqlc.arg(row_limit);
```

The plugin generates these control types (along with the query function and row type):

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SearchUsersSort {
    #[default]
    IdAsc,
    IdDesc,
    ShortestEmail,
}

#[derive(Debug, Clone, Default)]
pub struct SearchUsersParams<'a> {
    pub email: Option<&'a str>,
    pub phone: Option<&'a str>,
    pub row_limit: i64,
    pub has_orders: bool,
    pub sort: SearchUsersSort,
}
```

Call it using the generated params struct:

```rust
let users = queries::search_users(
    &pool,
    queries::SearchUsersParams {
        email: Some("alice@example.com"),
        has_orders: true,
        sort: queries::SearchUsersSort::IdDesc,
        row_limit: 50,
        ..Default::default()
    },
)
.await?;
```

This keeps the email and order-existence filters, omits the phone filter, and sorts by descending ID. Set `email: None` to omit the email filter too. Complete examples for every backend are in [`examples/dynamic-filter`](./examples/dynamic-filter/).

### Annotation placement

Here, a **structure** means a complete condition, `OR` operand, ordering term, or join.

- Place `:if` or `:flag` after the structure, after its opening parenthesis for multi-line expressions (`EXISTS ( -- :flag @has_orders`), or on the line immediately before it.
- Indent standalone annotation lines: sqlc drops comment lines that start in the first column.
- A standalone annotation gates the structure starting on the next line. Inside `AND (` this means the first `OR` operand, it falls back to the enclosing parenthesized structure only when no structure starts on the next line.
- An inline annotation covers every structure of the same clause entirely on that line. `LENGTH(email) ASC, id DESC -- :case @shortest_email` selects both ordering terms. Structures in nested subqueries on that line are untouched.
- A structure takes one annotation. Multiple names, such as `-- :if @start @end` or `-- :flag @active @visible`, require all named controls to be active.
- `:if` names must be SQL parameters. Use `:flag` for a control that is not a SQL parameter.

> [!IMPORTANT]
> Keep `LIMIT` or `;` on a line after the last annotation. sqlc drops trailing comments on the final line of a query, silently losing an annotation there. A standalone annotation also avoids this issue.

Conditional `sqlc.slice()` parameters become `Option<&[T]>`: `None` skips the structure, while `Some(&[])` keeps it and renders `NULL`, matching zero rows for an `IN` filter. Use `dynfilter::nilable(ids)` when an empty slice should mean "no filter".

#### Switch presets

`:switch` attaches to a clause: on the `WHERE` or `ORDER BY` keyword line, or standalone on the line before that keyword. Each clause takes at most one switch, and any number of clauses may carry one. Every declared choice needs at least one `-- :case @choice` in that clause, and a choice on several structures selects all of them together (a multi-term preset). Structures in a switched clause without an annotation stay unconditional (a tie-breaker `id ASC`), and `:if` or `:flag` structures can sit next to cases in a `WHERE`. The field is a control value, never a bind parameter: exactly one preset is always active, so a switched clause is never dropped. Map external strings to variants at the application boundary, the enum has no string conversion of its own.

### Gated joins

`:if` and `:flag` also attach to a complete `JOIN ... ON ...` (plain, `INNER`, `LEFT`, or `LEFT OUTER`) of a table, CTE, or aliased derived table: inline after the `ON` expression, or standalone on the line before the `JOIN` keyword for a multi-line join (indent the standalone line, sqlc drops comment lines that start in the first column). When the gate is off the whole join, its binds, and every other structure gated on the same names are omitted.

```sql
-- name: SearchUsersWithOrders :many
SELECT u.id, u.email, u.phone
FROM users u
JOIN orders o ON o.user_id = u.id AND o.created_at >= @orders_since -- :if @orders_since
WHERE
  u.phone <> '' -- :flag @with_phone
ORDER BY
  o.created_at DESC, -- :if @orders_since
  u.id ASC
;
```

`orders_since: None` runs `SELECT u.id, u.email, u.phone FROM users u ORDER BY u.id ASC` (plus the phone predicate when `with_phone` is set); `Some(..)` keeps the join, its bind, and the ordering term. The rules that keep the off-state SQL valid:

- A gated join owns its lines: it starts at the first non-blank character of its line and nothing but the annotation follows the `ON` expression on its last line. The renderer marks whole lines, so `FROM users u JOIN orders o ON ...` on one line is an error.
- Every reference to the joined relation outside the join must sit inside a structure gated on at least the join's names: a `WHERE` conjunct, `ORDER BY` term, `OR` operand, or later join. There is no implicit gating. A structure that references `o` must carry `-- :if @orders_since` itself. Gating it on a subset of the join's names is an error.
- The result's row shape must stay fixed. Projections cannot be gated, so a gated join's columns cannot be selected, and `SELECT *` in that query is an error. Select the base table's columns explicitly, as in the example above.
- `USING`, `NATURAL`, `CROSS`, `RIGHT`, `FULL`, comma joins, joins of parenthesized joins, table functions, unaliased derived tables, joins in `UPDATE ... FROM`, and `:case` on a join are not supported.

An inner join still multiplies rows per match. For pure filtering prefer `AND EXISTS (SELECT 1 FROM orders o WHERE o.user_id = u.id) -- :flag @has_orders`. Gate a join only when gated predicates or ordering need the joined columns.

<details>
<summary>Advanced join reference and name-resolution rules</summary>

- The plugin resolves references by SQL scope over the parsed query, so this covers `o.created_at`, `o.*`, `public.orders.created_at`, whole-row and table-valued uses such as `row_to_json(o)` or `assets_fts MATCH ...`, and unqualified columns of the joined relation (from the sqlc catalog, or from a CTE's or derived table's projection) anywhere in the owning scope, including inside parentheses, function arguments, and correlated subqueries. A subquery alias that shadows the gated alias refers to its own relation.
- A later `JOIN ... USING (c)` counts as a reference to `c` when the gated relation provides it. `DISTINCT ON`, `LIMIT`, `OFFSET`, `FETCH`, and `FOR UPDATE OF` targets are checked like any other clause.
- An output alias counts only when it is a whole term: before input columns in `ORDER BY`, after them in `GROUP BY`, and in `HAVING` only on SQLite and MySQL (PostgreSQL never resolves output aliases there). An alias inside an expression, such as `ORDER BY coalesce(created_at, '')`, is an input column.
- A column alias list (`JOIN orders o(oid, uid, stamp)`) renames the leading columns and keeps the rest. A bare name binds to a column of a visible relation before a whole-row use of a relation with that name. Each side of a `UNION`, `INTERSECT`, or `EXCEPT` is its own scope and may carry gated joins. The set operation's `ORDER BY` names output columns only.
- Identifiers fold per engine: PostgreSQL lowercases unquoted names and compares quoted ones exactly, so `JOIN orders O` and `"o".id` name the same relation while `"O".id` does not. SQLite and MySQL compare case-insensitively whether quoted or not.
- An unqualified column that may belong to a relation whose columns the plugin cannot see (a table missing from the sqlc catalog, such as a SQLite FTS5 virtual table, a derived table selecting `*`, or a table function) is an error when a gated relation could provide it. It must be qualified. An unqualified table name that exists in several schemas is an error for a gated join, and for any other table when the schemas disagree about the column, qualify the table with its schema.

</details>

### `OR` groups

A parenthesized disjunction that is a complete `WHERE` conjunct, at any depth including `EXISTS (...)` subqueries, exposes its top-level `OR` operands as structures. Each operand takes `:if` or `:flag` with the usual inline and standalone forms, the enclosing conjunct can still take its own annotation, and the runtime owns the `OR` separators.

```sql
-- name: SearchUsersByPattern :many
SELECT id, email, phone
FROM users
WHERE
  id > 0
  AND (
    email LIKE @email_pattern -- :if @email_pattern
    OR phone LIKE @phone_pattern -- :if @phone_pattern
  )
ORDER BY id
;
```

When every operand is annotated the group keeps an unconditional `FALSE` operand, so both patterns `None` renders `AND (FALSE)` and matches nothing. When at least one operand is unannotated nothing is inserted. Write `TRUE` as an operand for "match everything". A bare `WHERE a OR b` without parentheses, `:switch`/`:case` inside a group, and disjunctions in `ON`, `HAVING`, or projections are not supported. Keep the closing `)` on its own line. An annotation after `... OR phone LIKE @phone_pattern)` follows the conjunct, not the last operand, and gates the whole group.

### Filtering for `NULL`

To optionally select only rows whose `email` is `NULL`, use `IS NULL` with a flag:

```sql
-- name: FindUsersWithoutEmail :many
SELECT id, email FROM users
WHERE email IS NULL -- :flag @email_is_null
ORDER BY id;
```

`email_is_null: true` keeps the predicate; `false` omits it.

For "no filter", "filter by a value", and "filter for `NULL`", combine a flag with a nullable parameter and null-safe equality. `-- :if` cannot represent all three states because its `None` already means "omit the predicate". Ordinary equality also cannot match `NULL` by binding `NULL`.

```sql
-- name: FindUsers :many
SELECT id, email FROM users
WHERE email IS NOT DISTINCT FROM sqlc.narg(email) -- :flag @filter_email
ORDER BY id;
```

```rust
pub struct FindUsersParams<'a> {
    pub email: Option<&'a str>,
    pub filter_email: bool,
}
```

| `filter_email` | `email` | Behavior |
| -------------- | ------- | -------- |
| `false` | anything | No email filter |
| `true` | `Some("a@example.com")` | Match that email |
| `true` | `None` | Match `NULL` emails |

`sqlc.narg()` makes the parameter nullable. The flag independently controls whether the predicate and its bind are emitted. MySQL spells the null-safe operator `<=>`. Separate flags let multiple nullable filters operate independently within the same `WHERE` clause.

### Validation and limitations

Generation fails with the query name when:

- the SQL cannot be parsed for the selected engine
- an annotation keyword is unknown, or a `:switch` line is malformed: fewer than two choices, a duplicate choice, a missing `default=`, or a default that is not a choice
- `:if` names something that is not a SQL parameter, or a `:flag` name, switch field, or choice is a SQL parameter, or a `:flag` name is also a switch field or choice
- an annotation targets a projection, `GROUP BY`, `HAVING`, `LIMIT`, an operand of an unparenthesized `OR`, a sub-expression, or other fragment, an attachment is ambiguous, two annotations target one structure (including `:case` with `:if` or `:flag`), or a standalone annotation is not followed by a supported structure
- a gated join is a `USING`, `NATURAL`, `CROSS`, `RIGHT`, or `FULL` join or joins an unsupported relation form, shares a line with other SQL, sits in a `SELECT *` query, carries `:case`, names a table that exists in several schemas, or its relation or an unqualified column of it is referenced outside a structure gated on at least the join's names, or an unqualified column cannot be resolved because a relation in scope has unknown columns
- a `:switch` is not on or before a `WHERE`/`ORDER BY` keyword, two switches share a clause, a `:case` sits in a clause without a switch or names an undeclared choice, or a declared choice has no `:case`
- generated identifiers collide after Rust normalization: choices `id_asc` and `IdAsc`, a switch enum named like another generated item (including database enums and backend support types), or a control field named like a params field
- a control name cannot become a Rust identifier (`_`, `_1st`) or would become a reserved word such as `Self`
- `:if` names a parameter that sqlc reported more than once under the same name (ambiguous)
- an optional parameter is also bound somewhere its `-- :if` does not guard, for example in a `LIMIT` or in another structure, since the runtime would have nothing to bind for `None`
- a gated structure contains a multi-line string literal or quoted identifier, which cannot carry a condition marker. Multi-line block comments inside a gated structure are removed

### Runtime behavior

Queries without annotations are generated as usual. The `dynfilter` runtime module is only emitted when needed (including for slice expansion).

The generated `pub mod dynfilter` precompiles every dynamic query with `LazyLock` from the SQL constant plus a clause plan the plugin derives from the parse (header line, connector, and item lines per `WHERE`, `ORDER BY`, or `OR` group), so the runtime never inspects SQL keywords. A gated join has no clause of its own. Its lines carry the condition and are dropped whole. Switch cases lower to `dynfilter::Arg::Flag(matches!(params.sort, SearchUsersSort::IdAsc))` in the same bind slots a `:flag` uses. `build` drops inactive items, omits the connector of the first surviving item, drops a header whose items are all inactive, renumbers remaining placeholders, and returns a typed bind plan. Input and output placeholders follow the configured engine:

| Engine | Input placeholders | Output placeholders |
| ------ | ------------------ | ------------------- |
| PostgreSQL | `$N` | `$N` |
| SQLite | `?N` or `$N` | `$N` |
| MySQL | `?` by appearance | `?` |

Annotations and placeholders inside string literals, quoted identifiers, or comments are ignored. PostgreSQL dollar-quoted and escape strings, MySQL backslash escapes, SQLite bracket identifiers, and PostgreSQL nested block comments are supported.

Ordinary line comments and blank lines outside gated structures and dynamic clauses stay in the emitted SQL, so a query tag such as `-- app:checkout` remains visible in `pg_stat_statements`. A comment or blank line on a gated line or inside a `WHERE` or `ORDER BY` clause with dynamic items is dropped, since a gate marker or connector must end that line.

## Backend notes

Implementation details that differ per `db_crate`. The generated API is otherwise the same across backends.

### sqlx-postgres, sqlx-mysql, sqlx-sqlite

Examples: [`sqlx-postgres`](./examples/authors/sqlx-postgres/src/lib.rs), [`sqlx-mysql`](./examples/authors/sqlx-mysql/src/lib.rs), [`sqlx-sqlite`](./examples/authors/sqlx-sqlite/src/lib.rs)

Generated functions are async and take any `sqlx::Executor`.

> [!NOTE]
> SQLite uses dynamic typing. Columns with **NUMERIC affinity** may store values as **INTEGER** when they can be represented exactly as integers. For example, `13.0` may be stored as `13`. The generated code always reads NUMERIC as `f64` (`REAL`), so decoding can fail with a type mismatch when SQLite returns an integer. See the [SQLite type affinity docs](https://www.sqlite.org/datatype3.html) and the [`sqlx` type mapping docs](https://docs.rs/sqlx/latest/sqlx/sqlite/types/index.html) for details.

### rusqlite

Example: [`examples/authors/rusqlite`](./examples/authors/rusqlite/src/lib.rs)

Rusqlite functions are synchronous and accept connections, transactions, and savepoints through the generated trait:

```rust
pub trait RusqliteClient {
    fn connection(&self) -> &rusqlite::Connection;
}

pub fn get_author(
    client: &impl RusqliteClient,
    id: i64,
) -> rusqlite::Result<GetAuthorRow> { /* ... */ }
```

The trait is the extension point for wrappers that do not deref to `Connection`, such as pool guards: implement `connection()` for your type and every generated function accepts it. Execution queries (`:exec`, `:execrows`, `:execlastid`) step the statement to completion, so a DML statement with `RETURNING` succeeds and reports its write, `:execrows` returns `u64` from `changes()`.

### postgres

Example: [`examples/authors/postgres`](./examples/authors/postgres/src/lib.rs)

For `postgres`, generated functions are synchronous and take `&mut impl postgres::GenericClient`. Static queries expose `prepare_<fn>` and `<fn>_with`. `:many` also exposes `<fn>_iter`, returning `Result<postgres::RowIter<'_>, postgres::Error>`.

### tokio-postgres

Example: [`examples/authors/tokio-postgres`](./examples/authors/tokio-postgres/src/lib.rs)

For `tokio-postgres`, generated functions take `&impl tokio_postgres::GenericClient`. Static queries also expose preparation, reusable statements, and row streaming:

```rust
let statement = queries::prepare_get_author(&client).await?;
let author = queries::get_author_with(&client, &statement, id).await?;
let stream = queries::list_authors_stream(&client).await?;
```

### deadpool-postgres

Example: [`examples/authors/deadpool-postgres`](./examples/authors/deadpool-postgres/src/lib.rs)

`deadpool-postgres` generates the same API as `tokio-postgres` with `&impl deadpool_postgres::GenericClient`, implemented for pooled `Client` and `Transaction`, and prepares static SQL with `prepare_cached`.

### PostgreSQL drivers

These notes apply to `postgres`, `tokio-postgres`, and `deadpool-postgres`. All three support one-dimensional arrays only. Dynamic-filter queries do not expose `prepare_*` or `*_with` variants. `:execrows` and `:execresult` return `u64`.

On every PostgreSQL backend, including `sqlx-postgres`, sqlc infers untyped `LIMIT` and `OFFSET` parameters as `bigint`. Cast them explicitly (`::int` or `::bigint`) or add an override so the generated Rust parameter type matches. A bare `$1::int` loses sqlc's inferred parameter name, so use `sqlc.arg` when casting named parameters. Because `limit` and `offset` are SQL keywords, quote them in the macro:

```sql
LIMIT sqlc.arg('limit')::int OFFSET sqlc.arg('offset')::int
```

## Options

Set plugin options under `sql[].codegen[].options` in `sqlc.yaml`. All options are optional, the defaults generate SQLx PostgreSQL code in `queries.rs`.

```yaml
codegen:
  - plugin: sqlc-gen-rust
    out: src/
    options:
      db_crate: sqlx-postgres
      output: queries.rs
      query_parameter_limit: 1
```

The examples below show the `options` block to merge into that configuration. [Dynamic filter annotations](#dynamic-filters) work without enabling caching.

### `db_crate`

The backend used by the generated code. Default: `sqlx-postgres`. See [Supported crates](#supported-crates) for accepted values and their matching sqlc engines.

### `dynfilters`

Enable statement caching for dynamic queries with `dynfilters.prepared`:

```yaml
options:
  dynfilters:
    prepared: true
    prepared_skip: [ClearPhones] # optional: keep this query uncached
```

Skip lists use SQL query names (the name in `-- name: ClearPhones :exec`), not Rust function names.

| Option | Default | Meaning |
| ------ | ------- | ------- |
| `dynfilters.prepared` | `false` | Enumerate every dynamic query and emit variant constants and warm-ups where supported |
| `dynfilters.variant_limit` | `1024` | Maximum distinct variants per query, exceeding it is an error, never a sample. Raw control states (before deduplication) are also capped at 2^20 per query |
| `dynfilters.variants_skip` | `[]` | Dynamic queries excluded from enumeration, and therefore from caching and the bind-type check, each must be a dynamic query |
| `dynfilters.prepared_skip` | `[]` | Dynamic queries kept on the uncached path while still enumerated, independent of `dynfilters.variants_skip` |

#### Generated variants and caching

With `dynfilters.prepared` on, the plugin enumerates every SQL text each dynamic query can render, over every `:if` parameter (`Some`/`None`), every `:flag` (`true`/`false`), and every `:switch` choice, then deduplicates by rendered SQL, so a filter nested under an inactive `:flag` does not multiply the count. Gated `JOIN` clauses and `OR` groups are enumerated like any other control. For each eligible query the plugin emits `X_VARIANTS: &[&str]`, every SQL text the runtime can build, in the exact form the generated code executes (`$N` on PostgreSQL and SQLite, `?` on MySQL), an `X_STATES: &[dynfilter::State]` table mapping each control state to its variant text and bind plan, and on statement-caching backends a `prepare_x` warm-up plus an aggregate `prepare_dynfilter_variants` and `DYNFILTER_VARIANT_COUNT`. An eligible query's function looks its text and binds up in `X_STATES` instead of rendering them per call; a query with more than 128 distinct gates (condition sets) gets no table and keeps rendering. Eligible queries stop bypassing the backend's per-connection cache: sqlx drops `.persistent(false)`, rusqlite uses `prepare_cached`, and deadpool-postgres runs `prepare_cached` before each call.

| Backend | Caching | Emits |
| ------- | ------- | ----- |
| sqlx-postgres | yes, typed warm-up | constants, `prepare_x`, aggregate, count |
| sqlx-mysql, sqlx-sqlite | yes | constants, `prepare_x`, aggregate, count |
| rusqlite | yes | constants, `prepare_x`, aggregate, count |
| deadpool-postgres | yes | constants, `prepare_x(&ClientWrapper)`, aggregate, count |
| postgres, tokio-postgres | no statement cache | constants only |

#### Warming the cache

One `sqlc generate` produces the complete module. sqlc parses and types the base query. The variant texts come from the same renderer the generated code runs, so each constant is byte-for-byte what the query function executes. sqlc does not analyze every variant separately, so a variant can still fail when the database prepares it, and warming surfaces that preparation error while the connection is initialized rather than on the first request. Preparation is all a warm-up checks: execution, value-dependent encoding, and row decoding can still fail on the first call, and a variant that prepares is not thereby proven to decode into the base query's row type. Warming is optional, eligible queries also fill the cache on first use. The cache is per connection, so warm each connection as the pool creates it, after migrations have run:

```rust
use std::str::FromStr;

// sqlx: size the cache on the connect options, warm in `after_connect`.
let options = sqlx::postgres::PgConnectOptions::from_str(&url)?
    .statement_cache_capacity(queries::DYNFILTER_VARIANT_COUNT + 32);
let pool = sqlx::postgres::PgPoolOptions::new()
    .after_connect(|conn, _| Box::pin(queries::prepare_dynfilter_variants(conn)))
    .connect_with(options)
    .await?;

// rusqlite: the default cache holds 16 statements.
conn.set_prepared_statement_cache_capacity(queries::DYNFILTER_VARIANT_COUNT + 32);
queries::prepare_dynfilter_variants(&conn)?;

// deadpool-postgres: `post_create` hands the hook a `ClientWrapper`.
let pool = deadpool_postgres::Pool::builder(manager)
    .post_create(deadpool_postgres::Hook::async_fn(|client, _| {
        Box::pin(async move {
            queries::prepare_dynfilter_variants(client)
                .await
                .map_err(deadpool_postgres::HookError::Backend)
        })
    }))
    .build()?;
```

`DYNFILTER_VARIANT_COUNT` sums the eligible queries' variants and is a sizing input, not a guarantee: sqlx and rusqlite evict when the cache is smaller than what was warmed, and generated code never resizes a cache it does not own. Two queries can share a text, in which case the cache holds one entry for both. A retained prepared statement is not a precomputed execution plan on PostgreSQL. The server decides between generic and custom plans as usual.

#### Limits and PostgreSQL bind types

On SQLx PostgreSQL the cache must be enabled (`statement_cache_capacity` of at least `1`) in a package with the flag on: sqlx 0.8 gives every persistent statement a server-side name but only tracks it in an enabled cache, so with capacity `0` each warm-up or eligible execution leaks a named statement for the life of the connection. The generated warm-up detects the disabled cache after its first prepare and returns `sqlx::Error::Configuration`; eligible query functions accept any `Executor` and cannot check, so treat capacity `0` as unsupported there rather than as a cache that merely forgets. Skipped queries keep `.persistent(false)`, which never inserts into the cache and uses an unnamed statement on a miss. MySQL and SQLite close their uncached statements and are not affected.

A `WHERE` or `ORDER BY` clause that has dynamic items cannot contain a string literal, quoted identifier, or block comment spanning several lines; the runtime copies such continuation lines through verbatim. Move the value out of the clause or split it. Outside such clauses multi-line values are data and pass through untouched.

A query with a `sqlc.slice` parameter is never cache-eligible, even when the slice is conditional, because its element count changes the SQL text. It stays uncached, and its one-element expansion only feeds the bind-type check. PostgreSQL array parameters (`= ANY($1)`) stay eligible since their length never changes the text. Empty slices (`Some(&[])` renders `NULL`) are not enumerated.

On SQLx PostgreSQL the warm-up uses `prepare_with` and passes each variant's Rust bind types in bind order, resolved through overrides and nullability, because sqlx keys its cache by SQL text and an untyped `PREPARE` lets the server infer a type the later bind may not match (an `int4` column compared with an `i64` bind fails after an untyped warm-up with "incorrect binary data format"). Every control state that renders one text must therefore bind the same Rust types, and one text shared by two queries must too. sqlx looks a text up in the cache before it honours `persistent(false)`, so a skipped query still binds to a statement another query cached. The plugin therefore checks every source of a text in the module (cached, skipped, and static queries) and rejects a mismatch while any source still inserts into the cache. Resolve it by adding every dynamic query that shares the text to `dynfilters.prepared_skip`, or by changing the SQL when a static query is involved, since static queries always cache. Queries in other generated modules, hand-written SQL, and dynamic queries in `dynfilters.variants_skip` are outside this check. A custom encoder whose `Encode::produces()` depends on the value and disagrees with its `Type::type_info()` needs the skip as well.

### `query_parameter_limit`

The maximum number of parameters emitted as individual function arguments. The default is `1`. `0` always emits a params struct for parameterized queries. A params struct is emitted only when the parameter count is greater than this limit.

Params structs derive `Default` when all fields can be defaulted: optional and borrowed fields, primitives, `String`, `uuid::Uuid`, `serde_json::Value`, and override types marked `can_default: true`. String, bytes, and array params are borrowed. All other params, including non-copy override types, are stored by value.

### `overrides`

Customize Rust type mapping per column or database type. Each entry **must include exactly one** of the following: `column` or `db_type`.

- `column`: Override a specific table column (e.g. `users.metadata`)
- `db_type`: Override all columns of a given database type (e.g. `pg_catalog.varchar`)

Column overrides take precedence over database-type overrides.

For example, map `varchar` to `Cow`, but keep `authors.name` as `String`:

```yaml
options:
  overrides:
    - db_type: pg_catalog.varchar
      rs_type: std::borrow::Cow<'static, str>
      rs_slice: str
      can_default: true
    - column: .authors.name
      rs_type: String
      rs_slice: str
      can_default: true
```

`rs_type` is the Rust type used for returned values. Optional fields:

- `rs_slice`: the borrowed parameter view, `str` produces `&str` in this example.
- `copy_cheap`: marks the type as cheap to copy for parameter passing. Default: `false`.
- `can_default`: declares that the type implements `Default`, allowing params structs that contain it to derive `Default`. Default: `false`.

Column paths use `.{TableName}.{ColumnName}`. See the [attribute matching rules](#match-rules) for path matching.

### `row_attributes` / `column_attributes`

Inserts an arbitrary sequence of tokens immediately **before** the generated item that matches the path. Common usage includes adding Rust attributes (e.g. `#[derive(...)]`, `#[serde(...)]`, etc.).

The value accepts either a string or an array of strings. When using an array, items are concatenated with `\n`.

#### Match rules

Keys are treated as path segments separated by `.` and searched in the following order:

1. Full match
2. Suffix match (e.g., `.authors.id` -> `.id`)
3. Fallback `.`

#### `row_attributes`

`row_attributes` are searched with `.{QueryName}` (e.g., `.GetAuthor`), then fallback to `.`. Note that `row_attributes` effectively has only these two levels: `.{QueryName}` and `.`.

#### `column_attributes`

`column_attributes` are searched in two steps, and **Query-specific rules always win**:

1. Query scope: search with `.{QueryName}.{FieldName}`
2. Table scope (only if step 1 has no match): search with `.{TableName}.{ColumnName}`

Both steps use the same PathMap rules (full match -> suffix match -> `.`).

> `{FieldName}` is the **generated Rust field name** (snake_case), not necessarily the original SQL column name. For queries with duplicate column names (e.g. joins), generated fields may become `users_id`, `posts_id`, or even `id_1`, `id_2`. Check the generated `*Row` struct field names and use them in the key.

Example:

```yaml
options:
  row_attributes:
    .: "#[derive(Debug)]"
    .GetAuthor: "#[derive(Debug, serde::Serialize)]"
  column_attributes:
    .GetAuthor.name: '#[serde(rename = "displayName")]'
```

This gives every row `Debug`, adds `Serialize` to `GetAuthorRow`, and serializes its `name` field as `displayName`. Attribute values replace less-specific matches rather than accumulating, so the `GetAuthor` rule repeats `Debug`. Enable serde's `derive` feature in your application.

### `enum_derives`

Add additional items to the `#[derive(...)]` attribute for generated enums. Accepts an array of derive paths as strings.

```yaml
options:
  enum_derives:
    - serde::Serialize
    - serde::Deserialize
```

### `output`

Generated filename relative to sqlc's `out` directory. Default: `queries.rs`.

## Compatibility

The plugin is pre-1.0. Generated code and configuration options may change between minor versions. See [CHANGELOG.md](./CHANGELOG.md) for breaking changes and migration notes.

| Component | Tested with |
| --- | --- |
| Plugin build toolchain | Rust 1.89.0 with the `wasm32-wasip1` target (`rust-toolchain.toml`), protoc 29.3 |
| sqlc | 1.31.1 |
| sqlx | 0.8 |
| rusqlite | 0.32 |
| postgres | 0.19 |
| tokio-postgres | 0.7 |
| deadpool-postgres | 0.14 |

The pinned toolchain is only needed to build the plugin itself. Projects that consume the generated Rust need a compiler with `std::sync::LazyLock` (Rust 1.80 or newer) and one of the backend crate versions above. The example crates use edition 2024.

## Credits

The core plugin and type mapping come from [tunamaguro/sqlc-gen-rust](https://github.com/tunamaguro/sqlc-gen-rust).

The `-- :if` dynamic filter annotation is modeled on the same feature in [sqlc-dev/sqlc-gen-go](https://github.com/sqlc-dev/sqlc-gen-go).

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](./LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](./LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

Generated output is your own code. When a query uses dynamic filters, the plugin inlines its `dynfilter` runtime from `src/db_crates/dynfilter_runtime.rs` into the generated file. That copied code stays under the same MIT OR Apache-2.0 terms, and either license permits shipping it inside a project under a different license as long as that license's notice requirements are met.

## Contribution

See [CONTRIBUTING.md](./CONTRIBUTING.md) for the development setup and pull-request checklist, and [SECURITY.md](./SECURITY.md) for reporting vulnerabilities.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.

# sqlc-gen-rust

sqlc plugin for Rust database crates.

## Usage

Add the following to your configuration file to use this plugin.

```yaml
version: "2"
plugins:
  - name: sqlc-gen-rust
    wasm:
      url: https://github.com/orlandoc01/sqlc-gen-rust/releases/download/v0.1.0/sqlc-gen-rust.wasm
      sha256: 81f73597a3d4a247b615c504cf7e0dad388ac3fe656e443ccf8ad74d212d1223
sql:
  - schema: schema.sql
    queries: queries.sql
    engine: postgresql
    codegen:
      - plugin: sqlc-gen-rust
        out: src/
```

## Supported crates

- [postgres](https://crates.io/crates/postgres)
- [tokio-postgres](https://crates.io/crates/tokio-postgres)
- [deadpool-postgres](https://crates.io/crates/deadpool-postgres)
- [sqlx-postgres](https://docs.rs/sqlx/latest/sqlx/postgres/index.html)
- [sqlx-mysql](https://docs.rs/sqlx/latest/sqlx/mysql/index.html)
- [sqlx-sqlite](https://docs.rs/sqlx/latest/sqlx/sqlite/index.html)
- [rusqlite](https://docs.rs/rusqlite/latest/rusqlite/)

> [!NOTE]
> SQLite uses dynamic typing. Columns with **NUMERIC affinity** may store values as **INTEGER** when they can be represented exactly as integers. 
> For example, `13.0` may be stored as `13`. The generated code always reads NUMERIC as `f64` (`REAL`), so decoding can fail with a type mismatch when SQLite returns an integer. See the [SQLite type affinity docs](https://www.sqlite.org/datatype3.html) and the [`sqlx` type mapping docs](https://docs.rs/sqlx/latest/sqlx/sqlite/types/index.html) for details.


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

### Using generated code

```rust
mod queries;

use queries::{CreateAuthor, DeleteAuthor, ListAuthors};

#[tokio::main]
async fn main() {
    let (client, conn) = tokio_postgres::connect(
        &std::env::var("DATABASE_URL").unwrap(),
        tokio_postgres::NoTls,
    )
    .await
    .unwrap();
    tokio::spawn(async move {
        if let Err(e) = conn.await {
            panic!("connection error: {e}");
        }
    });

    // list authors
    let authors = ListAuthors.query_many(&client).await.unwrap();
    assert_eq!(authors.len(), 0);
    // let author_stream = ListAuthors.query_stream(&client).await.unwrap(); // stream of rows

    // crate and get an author (INSERT ... RETURNING ...)
    let author = {
        let binding = CreateAuthor::builder()
            .name("John")
            .bio(Some("Foo"))
            .build();

        // let binding = CreateAuthor::builder().name("John").build(); // missing field won't compile

        binding.query_one(&client).await.unwrap()
        //  binding.query_opt(&client).await.unwrap() // this returns Option<T>
    };
    assert_eq!(author.id, 0);

    // delete author
    let affected_row = DeleteAuthor::builder()
        .id(0)
        .build()
        .execute(&client)
        .await
        .unwrap();
    assert_eq!(affected_row, 1);
}
```

See below for examples with other supported crates.

- [`postgres` generated code](./examples/authors/postgres/src/lib.rs)
- [`tokio-postgres` generated code](./examples/authors/tokio-postgres/src/lib.rs)
- [`deadpool-postgres` generated code](./examples/authors/deadpool-postgres/src/lib.rs)
- [`sqlx-postgres` generated code](./examples/authors/sqlx-postgres/src/lib.rs)
- [`sqlx-mysql` generated code](./examples/authors/sqlx-mysql/src/lib.rs)
- [`sqlx-sqlite` generated code](./examples/authors/sqlx-sqlite/src/lib.rs)
- [`rusqlite` generated code](./examples/authors/rusqlite/src/lib.rs)

## Supported Features

### Query Annotations

| crate             | `:exec` | `:execlastid` | `:many` | `:one` | `:copyfrom` |
| ----------------- | ------- | ------------- | ------- | ------ | ------------ |
| postgres          | ✅       | ❌             | ✅       | ✅      | ❌            |
| tokio-postgres    | ✅       | ❌             | ✅       | ✅      | ✅            |
| deadpool-postgres | ✅       | ❌             | ✅       | ✅      | ✅            |
| sqlx-postgres     | ✅       | ❌             | ✅       | ✅      | ✅            |
| sqlx-mysql        | ✅       | ❌             | ✅       | ✅      | ❌            |
| sqlx-sqlite       | ✅       | ❌             | ✅       | ✅      | ❌            |

### Macros

| Macro        | Status |
| ------------ | ------ |
| `sqlc.arg`   | ✅      |
| `sqlc.embed` | ✅      |
| `sqlc.narg`  | ✅      |
| `sqlc.slice` | ✅      |

### `sqlc.embed`

`sqlc.embed(table)` emits a nested model struct for the table and decodes its columns by their
SELECT position. See the [embed examples](./examples/embed/) for joined rows.

### `sqlc.slice`

The builder API retains upstream slice expansion. SQLite queries using named (`@name`) parameters
emit numbered slice markers that the builder API cannot expand; use `api: params_struct` instead.

## Options

### `db_crate`

The crate used in the generated code. Default is `tokio-postgres`. Available values are below.

- `postgres` 
- `tokio-postgres`
- `deadpool-postgres`
- `sqlx-postgres`
- `sqlx-mysql`
- `sqlx-sqlite`
- `rusqlite`

### `api`

Select the generated query API. `builder` is the default and preserves the existing generated
builder structs. `params_struct` is available for `sqlx-postgres`, `sqlx-mysql`, and
`sqlx-sqlite`; it generates SQL constants, free async functions, and public params/row structs
instead of query builders. `:copyfrom` and `:batch*` are not supported with this API.
For SQLite and MySQL `sqlc.slice()` queries, it also emits `pub mod dynfilter` and uses its
runtime bind plan so numbered slice markers are expanded and rebound correctly. PostgreSQL
keeps array binding.

```yaml
options:
  db_crate: sqlx-sqlite
  api: params_struct
```

For example, a `:one` query with one `id` parameter generates a direct argument, while a query
with two parameters generates a params struct:

```rust
pub const GET_AUTHOR: &str = "SELECT id, name FROM authors WHERE id = ?";

pub struct GetAuthorRow {
    pub id: i64,
    pub name: String,
}

pub async fn get_author<'e>(
    executor: impl sqlx::Executor<'e, Database = sqlx::Sqlite>,
    id: i64,
) -> Result<GetAuthorRow, sqlx::Error> { /* ... */ }

#[derive(Debug, Clone, Default)]
pub struct CreateAuthorParams<'a> {
    pub name: &'a str,
    pub bio: Option<&'a str>,
}
```

#### Dynamic filters with `-- :if`

With `api: params_struct`, any SQL line annotated with `-- :if @param` becomes
runtime-selectable. No extra option is needed: queries without annotations are generated
as usual, and the `dynfilter` runtime module is only emitted when a query uses it.

Conditional SQL parameters become `Option<T>` (`None` skips the line), and names that
appear only in an annotation become appended `bool` fields. Conditional `sqlc.slice()`
parameters become `Option<&[T]>`: `None` skips the line, while `Some(&[])` keeps it and
renders `NULL`, matching zero rows. Use `dynfilter::nilable(ids)` when an empty slice
should mean "no filter".

```sql
-- name: SearchUsers :many
SELECT * FROM users
WHERE TRUE
  AND email = @email -- :if @email
  -- :if @phone
  AND phone = @phone
  AND EXISTS ( -- :if @has_orders
    SELECT 1 FROM orders WHERE orders.user_id = users.id
  )
ORDER BY
  id ASC,  -- :if @id_asc
  id DESC, -- :if @id_desc
  TRUE;
```

```rust
pub struct SearchUsersParams<'a> {
    pub email: Option<&'a str>,
    pub phone: Option<&'a str>,
    pub has_orders: bool,
    pub id_asc: bool,
    pub id_desc: bool,
}

let ids = dynfilter::nilable(&ids);
```

Inline annotations drop their own line. A standalone annotation drops the next line;
when that line opens a parenthesized block, the whole block is dropped. Multiple names on
one annotation require every parameter to be active. An annotation must be last on its
line.

The generated `pub mod dynfilter` precompiles every dynamic query with `LazyLock`, drops
inactive segments, renumbers remaining placeholders, and returns a typed bind plan. Input
and output placeholders follow the configured engine:

| Engine | Input placeholders | Output placeholders |
| ------ | ------------------ | ------------------- |
| PostgreSQL | `$N` | `$N` |
| SQLite | `?N` or `$N` | `$N` |
| MySQL | `?` by appearance | `?` |

Annotations and placeholders inside string literals, quoted identifiers, or comments are
ignored. PostgreSQL dollar-quoted and escape strings, MySQL backslash escapes, SQLite
bracket identifiers, and PostgreSQL nested block comments are supported.

### `query_parameter_limit`

The maximum number of parameters emitted as individual function arguments with
`api: params_struct`. The default is `1`; `0` always emits a params struct for parameterized
queries. A params struct is emitted only when the parameter count is greater than this limit.

Params structs always derive `Default`. Override types used as params must therefore implement
`Default`. String, bytes, and array params are borrowed; all other params, including non-copy
override types, are stored by value.

### `overrides`

Customize Rust type mapping per column or database type. Each entry **must include exactly one** of the following: `column` or `db_type`.

- `column`: Override a specific table column (e.g. `users.metadata`)
- `db_type`: Override all columns of a given database type (e.g. `pg_catalog.varchar`)

When both are specified, it will result in an error. Furthermore, entries with a `column` key are always prioritized over `db_type` overrides.

The following is an example configuration:

```yaml
sql:
  - schema: examples/e-commerce/schema.sql
    queries: examples/e-commerce/queries.sql
    engine: postgresql
    codegen:
      - plugin: sqlc-gen-rust
        out: examples/e-commerce/src
        options:
          output: sqlx_query.rs
          db_crate: sqlx-postgres
          overrides:
            - db_type: pg_catalog.varchar # Database type to override
              rs_type: std::borrow::Cow<'static,str>  # Rust type to use in generated code
              rs_slice: str # Optional. If set, the argument of the generated code uses `&str` instead of `&std::borrow::Cow<'static,str>`
              copy_cheap: false # Optional. If true, the argument of the generated code uses `std::borrow::Cow<'static,str>` instead of `&std::borrow::Cow<'static,str>`.
            - column: .users.created_at # A column name to override. This will be searched for in the `.{TableName}.{ColumnName}` path. For details about matching columns see `row_attributes` / `column_attributes` below
              rs_type: serde_json::Value
```

### `row_attributes` / `column_attributes`

Inserts an arbitrary sequence of tokens immediately **before** the generated item that matches the path.
Common usage includes adding Rust attributes (e.g. `#[derive(...)]`, `#[serde(...)]`, etc.).

The value accepts either a string or an array of strings. When using an array, items are concatenated with `\n`.

#### Match Rules

Keys are treated as path segments separated by `.` and searched in the following order:

1. Full match
2. Suffix match (e.g., `.authors.id` -> `.id`)
3. Fallback `.`

#### `row_attributes`

`row_attributes` are searched with `.{QueryName}` (e.g., `.GetAuthor`), then fallback to `.`.
Note that `row_attributes` effectively has only these two levels: `.{QueryName}` and `.`.

#### `column_attributes`

`column_attributes` are searched in two steps, and **Query-specific rules always win**:

1. Query scope: search with `.{QueryName}.{FieldName}`
2. Table scope (only if step 1 has no match): search with `.{TableName}.{ColumnName}`

Both steps use the same PathMap rules (full match -> suffix match -> `.`).

> `{FieldName}` is the **generated Rust field name** (snake_case), not necessarily the original SQL column name.
> For queries with duplicate column names (e.g. joins), generated fields may become `users_id`, `posts_id`, or even `id_1`, `id_2`.
> Check the generated `*Row` struct field names and use them in the key.

Examples

```yaml
sql:
    codegen:
      - plugin: sqlc-gen-rust
        out: examples/authors/tokio-postgres/src
        options:
          output: queries.rs
          db_crate: tokio-postgres
          row_attributes:
            .: "#[doc=\"apply to all row\"]"
            .GetAuthor: "#[doc=\"apply to only GetAuthorRow\"]"
          column_attributes:
            .: "#[doc=\"apply to all column\"]"
            .name: "#[doc=\"apply to all name column\"]"
            .author.id: "#[doc=\"apply to author table's id column\"]"
            .GetAuthor.id: "#[doc=\"apply to GetAuthor's id column\"]"
```

### `enum_derives` 

Add additional items to the `#[derive(...)]` attribute for generated enums.
Each field accepts an array of derive paths as strings. 

```yaml
sql:
  - codegen:
      - plugin: sqlc-gen-rust
        out: examples/e-commerce/src
        options:
          enum_derives: 
            - serde::Serialize
            - serde::Deserialize
```

### `output`

Generated code destination. Default is `queries.rs`.

## Credits

This project is a fork of [tunamaguro/sqlc-gen-rust](https://github.com/tunamaguro/sqlc-gen-rust),
which provides the core plugin, type mapping, and query builder API. The `params_struct` API and
`-- :if` dynamic filters were added in this fork.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](./LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](./LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

## Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.

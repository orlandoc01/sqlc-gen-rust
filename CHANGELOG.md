# Changelog

All notable changes to this project are documented in this file.

## [0.2.0] - 2026-09-18

### Breaking changes

- `-- :if` dynamic filters are now structural. An annotation attaches to one complete `WHERE`
  conjunct, including parenthesized predicates and `EXISTS (...)` in any subquery, or one
  complete `ORDER BY` term. The plugin parses SQL with the configured engine dialect, the
  runtime owns separators, and any other annotated structure is a generation error naming the
  query.

- `-- :if` is now exclusive to optional SQL parameters. A name that is not a SQL parameter is a
  generation error pointing at `-- :flag @name`, which gates a structure on a generated
  `pub name: bool` field. `-- :flag` on a SQL parameter is an error pointing back at `-- :if`.
- Unknown annotation keywords after `-- :` (for example `-- :sort`) are generation errors.
- A `WHERE` or `ORDER BY` clause with dynamic items cannot contain a literal, quoted
  identifier, or block comment spanning several lines; the runtime would copy the
  continuation lines through unlexed. Move the value out of the clause or split it.
- The inlined runtime's `compile_with_arg_order` takes the engine `Dialect` beside
  `Placeholders` instead of inferring lexing rules from the placeholder form.
- sqlc parameter numbers must be contiguous from 1; a gap or duplicate is a generation error
  instead of silently gating the wrong argument.

### Added

- `-- :if` and `-- :flag` on a complete `JOIN ... ON ...` (plain, `INNER`, `LEFT`, `LEFT OUTER`)
  of a table, CTE, or aliased derived table (including `LATERAL`) omit the whole join when the
  gate is off. The join must own its lines, and every reference to the joined relation must sit
  in a structure gated on at least the join's names, so the off-state SQL stays valid. The
  plugin resolves references by SQL scope over the parsed query: `alias.column`, `alias.*`,
  `schema.table.column`, whole-row and table-valued uses of the alias, and unqualified columns
  of the joined relation anywhere in the owning scope, including inside parentheses, function
  arguments, correlated subqueries, `DISTINCT ON`, `LIMIT`/`OFFSET`/`FETCH`, and `FOR UPDATE
  OF` targets. An output alias shadows an input column only as a whole term, per engine rules,
  and column alias lists (`o(oid, uid)`) rename the leading columns. Identifiers fold per engine (PostgreSQL lowercases
  unquoted names and compares quoted ones exactly; SQLite and MySQL compare
  case-insensitively). `SELECT *`, `USING`, `NATURAL`, `CROSS`, `RIGHT`, `FULL`, other relation
  forms, `:case` on a join, an unqualified column that may belong to a relation the catalog
  does not describe, and an unqualified table name that exists in several schemas are
  generation errors. Queries that the earlier token-based check accepted, such as
  `WHERE (created_at > '2024')` or `row_to_json(o)` beside a gated `orders o`, now fail
  generation; qualify the column and gate its structure, or drop the reference.
- `-- :if` and `-- :flag` on the operands of a parenthesized `OR` group that is a complete
  `WHERE` conjunct. The runtime owns the `OR` separators; when every operand is annotated, an
  unconditional `FALSE` operand keeps an all-inactive group valid and matching nothing.
- `-- :switch @field a b [c ...] default=a` on a `WHERE` or `ORDER BY` keyword line (or standalone
  on the line before it) plus `-- :case @a` on each alternative structure generate one
  `<Query><Field>` enum with `#[default]` and a non-optional params field. Exactly one preset is
  always active; a choice on several structures selects them together, unannotated structures
  stay unconditional, and `:if`/`:flag` structures can sit next to cases in a `WHERE`. Cases
  lower to `dynfilter::Arg::Flag(matches!(..))` in the same bind slots flags use, so the runtime
  is unchanged.
- `dynfilters.prepared: true` enumerates every distinct SQL shape of each dynamic query in memory
  during the single `sqlc generate` pass and keeps each shape of a cache-eligible query as a
  prepared statement: eligible queries emit `X_VARIANTS` with runtime-exact SQL, an `X_STATES`
  control-state table the query function looks its text and bind plan up in instead of
  rendering per call, a `prepare_x` warm-up, an aggregate `prepare_dynfilter_variants`, and
  `DYNFILTER_VARIANT_COUNT`, and stop bypassing the statement cache on sqlx, rusqlite, and
  deadpool-postgres (postgres and tokio-postgres get constants only). SQLx PostgreSQL warms with `prepare_with` and the Rust bind
  types, and rejects one SQL text bound with different types. `dynfilters.variant_limit` (default
  `1024`) caps distinct variants per query, enumeration stops at 2^20 raw control states before
  dedup, and `dynfilters.variants_skip` excludes named queries from enumeration. Queries with
  `sqlc.slice` parameters stay uncached; `dynfilters.prepared_skip` opts others out of caching
  without excluding them from enumeration.

### Migration from v0.1.x

Remove legacy `WHERE TRUE`, `AND TRUE`, and trailing `TRUE` sentinels:

```sql
-- Before
SELECT * FROM users
WHERE TRUE
  AND email = @email -- :if @email
  AND TRUE
ORDER BY
  id ASC, -- :if @id_asc
  TRUE;

-- After
SELECT * FROM users
WHERE
  email = @email -- :if @email
ORDER BY
  -- :if @id_asc
  id ASC
;
```

Keep inline annotations on the line where their structure ends. Put an annotation for a
multi-line parenthesized structure after its opening `(` or on a standalone line before it. A
standalone annotation now attaches to the complete structure starting on the next line, not just
the next physical line. A terminating `;` cannot follow an inline annotation, so put it on its
own line. Because sqlc drops a trailing comment on a query's final line, use the standalone form
for a final conditional conjunct.

Rename annotation-only names from `:if` to `:flag`, and replace groups of mutually exclusive
flags with a switch:

```sql
-- Before
  AND EXISTS ( -- :if @has_orders
    SELECT 1 FROM orders WHERE orders.user_id = users.id
  )
ORDER BY
  id ASC, -- :if @id_asc
  id DESC -- :if @id_desc
LIMIT sqlc.arg(row_limit);

-- After
  AND EXISTS ( -- :flag @has_orders
    SELECT 1 FROM orders WHERE orders.user_id = users.id
  )
ORDER BY -- :switch @sort id_asc id_desc default=id_asc
  id ASC, -- :case @id_asc
  id DESC -- :case @id_desc
LIMIT sqlc.arg(row_limit);
```

```rust
// Before
SearchUsersParams { has_orders: true, id_desc: true, ..Default::default() }
// After
SearchUsersParams { has_orders: true, sort: SearchUsersSort::IdDesc, ..Default::default() }
```

Conditional parameters stay `Option<T>`, `:flag` names stay `bool` fields, and conditional
slices stay `Option<&[T]>`. In the generated `dynfilter`
runtime, `nilable`, `Bind`, `Arg`, and placeholder handling for each engine are unchanged.
`dynfilter::compile_with_arg_order` now takes the clause plan the plugin emits
(`&[dynfilter::Clause]`, with the new `Clause` and `Connector` types) instead of re-deriving
clause structure from the SQL text; generated code is the only expected caller. The order-less
`dynfilter::compile` and `Compiled::arg_count` were removed: generated code never called them. `sqlparser` is
now a build dependency of the plugin, not generated code.

### Fixed

- The generated runtime scanned PostgreSQL brackets as quoted identifiers, so `ARRAY[$1, $2]`,
  `vals[$1 + 1]`, and slices lost their binds; it read `$9` inside an identifier such as
  `price$9` as a parameter; it did not know MySQL `#` comments, so a `?` in one claimed an
  argument; and it honoured a `-- :if $N` marker quoted in ordinary comment prose. Brackets
  are opaque only for SQLite, unquoted identifiers are consumed whole, `#` comments and MySQL's
  `-- ` rule are recognized, and the plugin drops ordinary line comments from gated lines and
  from `WHERE`/`ORDER BY` clauses with dynamic items, so a marker can never follow prose.
  Comments and blank lines elsewhere in an annotated query stay in the emitted SQL, so query
  tags such as `-- app:checkout` remain visible to the server.
- Columns named like a clause keyword the dialect does not reserve (`returning`, `fetch`,
  `window` on MySQL and SQLite) no longer cut a dynamic `WHERE` or `ORDER BY` short; the
  clause extent comes from the parsed items.
- `Compiled::build` no longer allocates hash maps per call, and variant enumeration
  deduplicates control states by active gates for any number of gates, so a query past the
  128-gate state-table limit renders each distinct shape once.
- Unknown keys inside `dynfilters:` are now generation errors instead of being ignored.

- MySQL `YEAR` columns map to `u16`; they previously generated the nonexistent type `int16`.
- Annotation discovery now uses the engine's own tokenizer, so backslashes in SQLite and plain
  PostgreSQL strings, MySQL escaped quotes, annotations after a multi-line literal or comment
  closes, and `-- :` text inside ordinary comments are all handled correctly. Ordinary comments
  that merely mention a directive are inert.
- Control bind slots are allocated after every SQL argument even when sqlc reports several
  parameters under one name; previously a duplicate name could shift a flag onto a parameter.
- The generated runtime no longer panics on non-ASCII characters in SQL comments.
- The generated PostgreSQL runtime no longer mistakes the JSON operators `?`, `?|`, and `?&` for
  bind placeholders when a query has a dynamic filter; only MySQL and SQLite recognise a bare
  `?` as a bind.
- `sqlc.slice` markers are numbered by SQL token, so text such as `/*SLICE:ids*/?` inside a
  string literal, quoted identifier, or comment is left byte-for-byte unchanged instead of
  gaining a digit.
- A malformed directive such as `-- :` or `-- :!` is a generation error naming the query and
  line; previously it could crash the plugin.
- Generated SQLite SQL constants no longer start with the preceding query's own-line `;`,
  which sqlc's SQLite engine copies onto the next query's text.

### Newly rejected query forms

These were silently miscompiled before and now fail generation with the query name: an optional
parameter bound outside the structure its `-- :if` guards (for example in `LIMIT`), a gated
structure containing a multi-line string literal, `-- :if` on a parameter name sqlc reported
twice, control names that cannot become Rust identifiers (`_`, `_1st`, `self` as a choice), and
a switch enum whose name collides with a database enum or a backend support type.

## [0.1.1] - 2026-09-13

- See the [GitHub releases](https://github.com/orlandoc01/sqlc-gen-rust/releases).

## [0.1.0] - 2026-09-13

- See the [GitHub releases](https://github.com/orlandoc01/sqlc-gen-rust/releases).

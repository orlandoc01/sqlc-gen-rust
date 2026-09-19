use super::*;
use crate::dynfilter::test_support::{catalog, compile, params, parse_uncatalogued, table};
use crate::dynfilter_runtime::{Arg, Placeholders};

fn parse(sql: &str, dialect: Dialect) -> Result<String, String> {
    crate::dynfilter::strict::parse(sql, &params(&[]), dialect, &catalog())
        .map(|info| info.expect("annotated query").annotated_sql)
}

fn error(sql: &str, dialect: Dialect) -> String {
    parse(sql, dialect).expect_err(sql)
}

/// The generated SQL with the join's flag off and on.
fn states(sql: &str, dialect: Dialect, catalog: &Catalog) -> (String, String) {
    let info = crate::dynfilter::strict::parse(sql, &params(&[]), dialect, catalog)
        .unwrap()
        .unwrap();
    let compiled = compile(&info, Placeholders::Numbered, &[]);
    (
        compiled.build(&[Arg::Flag(false)]).0,
        compiled.build(&[Arg::Flag(true)]).0,
    )
}

const JOIN: &str = "SELECT u.id\nFROM users u\nJOIN orders o ON o.user_id = u.id -- :flag @x\n";

#[test]
fn resolves_columns_inside_parentheses_and_function_arguments() {
    for clause in [
        "WHERE (created_at > '2024')",
        "ORDER BY coalesce(created_at, '')",
        "WHERE u.id + created_at > 0",
        "GROUP BY created_at",
        "HAVING count(created_at) > 0",
    ] {
        let error = error(&format!("{JOIN}{clause}"), Dialect::PostgreSql);
        assert!(
            error.contains("column `created_at` of `o` is referenced unqualified on line 4"),
            "{clause}\n{error}"
        );
    }
    let (off, on) = states(
        &format!(
            "{JOIN}WHERE\n  u.id > 0\n  AND (created_at > '2024') -- :flag @x\nORDER BY\n  coalesce(created_at, '') DESC, -- :flag @x\n  u.id ASC"
        ),
        Dialect::PostgreSql,
        &catalog(),
    );
    assert_eq!(
        off,
        "SELECT u.id\nFROM users u\nWHERE\n  u.id > 0\nORDER BY\n  u.id ASC"
    );
    assert_eq!(
        on,
        "SELECT u.id\nFROM users u\nJOIN orders o ON o.user_id = u.id\nWHERE\n  u.id > 0\n  AND (created_at > '2024')\nORDER BY\n  coalesce(created_at, '') DESC,\n  u.id ASC"
    );
}

#[test]
fn resolves_qualified_wildcards_schema_paths_and_whole_row_references() {
    for (sql, expected) in [
        (
            "SELECT u.id, public.orders.created_at\nFROM users u\nJOIN public.orders ON orders.user_id = u.id -- :flag @x",
            "alias `orders` is referenced on line 1",
        ),
        (
            "SELECT u.id, orders.created_at\nFROM users u\nJOIN public.orders ON orders.user_id = u.id -- :flag @x",
            "alias `orders` is referenced on line 1",
        ),
        (
            "SELECT u.id, public.orders.created_at\nFROM users u\nJOIN orders ON orders.user_id = u.id -- :flag @x",
            "alias `orders` is referenced on line 1",
        ),
        (
            &format!("SELECT u.id, row_to_json(o)\n{}", &JOIN[12..]),
            "alias `o` is referenced on line 1",
        ),
        (
            &format!("SELECT u.id, o.*\n{}", &JOIN[12..]),
            "alias `o` is referenced on line 1",
        ),
        (
            &format!("SELECT u.id, count(o.*)\n{}", &JOIN[12..]),
            "alias `o` is referenced on line 1",
        ),
        (
            &format!("SELECT u.id, (o).created_at\n{}", &JOIN[12..]),
            "alias `o` is referenced on line 1",
        ),
    ] {
        let error = error(sql, Dialect::PostgreSql);
        assert!(error.contains(expected), "{sql}\n{error}");
    }
    assert!(
        parse(
            &format!("SELECT u.id, count(*)\n{}", &JOIN[12..]),
            Dialect::PostgreSql
        )
        .is_ok()
    );
    assert!(
            parse(
                "SELECT u.id, sales.orders.created_at\nFROM users u\nJOIN sales.orders so ON so.user_id = u.id\nJOIN public.orders ON orders.user_id = u.id -- :flag @x",
                Dialect::PostgreSql
            )
            .is_ok()
        );
}

#[test]
fn folds_identifiers_per_engine() {
    let mixed = "SELECT u.id, \"o\".created_at\nFROM users u\nJOIN orders O ON O.user_id = u.id -- :flag @x";
    let quoted_upper = "SELECT u.id\nFROM users u\nJOIN orders O ON O.user_id = u.id -- :flag @x\nWHERE \"O\".id > 0";
    let backticks = "SELECT u.id\nFROM users u\nJOIN orders `O` ON `O`.user_id = u.id -- :flag @x\nWHERE o.id > 0";
    assert!(error(mixed, Dialect::PostgreSql).contains("alias `O` is referenced on line 1"));
    assert!(parse(quoted_upper, Dialect::PostgreSql).is_ok());
    assert!(error(quoted_upper, Dialect::Sqlite).contains("alias `O` is referenced on line 4"));
    assert!(error(mixed, Dialect::Sqlite).contains("alias `O` is referenced on line 1"));
    assert!(error(backticks, Dialect::MySql).contains("alias `O` is referenced on line 4"));
    let column = "SELECT u.id\nFROM users u\nJOIN orders o ON o.user_id = u.id -- :flag @x\nORDER BY \"Created_At\"";
    assert!(parse(column, Dialect::PostgreSql).is_ok());
    assert!(error(column, Dialect::Sqlite).contains("column `Created_At` of `o`"));
}

#[test]
fn resolves_nested_scopes_with_local_shadowing() {
    let shadowed =
        format!("{JOIN}WHERE EXISTS (SELECT 1 FROM users o WHERE o.id = u.id AND email <> '')");
    assert!(parse(&shadowed, Dialect::PostgreSql).is_ok());
    let sibling =
        format!("{JOIN}WHERE u.id IN (SELECT o.user_id FROM orders o WHERE o.created_at > '2024')");
    assert!(parse(&sibling, Dialect::PostgreSql).is_ok());
    let correlated = format!("{JOIN}WHERE EXISTS (SELECT 1 FROM users x WHERE x.id = o.user_id)");
    assert!(error(&correlated, Dialect::PostgreSql).contains("alias `o` is referenced on line 4"));
    let bare_in_subquery =
        format!("{JOIN}WHERE EXISTS (SELECT 1 FROM users x WHERE created_at > '2024')");
    assert!(
        error(&bare_in_subquery, Dialect::PostgreSql)
            .contains("column `created_at` of `o` is referenced unqualified on line 4")
    );
    let gated = format!(
        "{JOIN}WHERE\n  u.id > 0\n  AND EXISTS ( -- :flag @x\n    SELECT 1 FROM users x WHERE x.id = o.user_id AND created_at > '2024'\n  )"
    );
    assert!(parse(&gated, Dialect::PostgreSql).is_ok());
}

#[test]
fn resolves_ctes_derived_tables_and_schemas() {
    let cte = "WITH orders AS (SELECT id, 'x' AS note FROM users)\nSELECT u.id\nFROM users u\nJOIN orders o ON o.id = u.id -- :flag @x\nWHERE note = 'x'";
    assert!(
        error(cte, Dialect::PostgreSql)
            .contains("column `note` of `o` is referenced unqualified on line 5")
    );
    let cte_shadows_catalog = "WITH orders AS (SELECT id FROM users)\nSELECT u.id\nFROM users u\nJOIN orders o ON o.id = u.id -- :flag @x\nWHERE created_at > '2024'";
    assert!(parse(cte_shadows_catalog, Dialect::PostgreSql).is_ok());
    let derived = "SELECT u.id\nFROM users u\nJOIN (SELECT user_id AS uid FROM orders) o ON o.uid = u.id -- :flag @x\nWHERE uid > 0";
    assert!(
        error(derived, Dialect::PostgreSql)
            .contains("column `uid` of `o` is referenced unqualified on line 4")
    );
    let derived_star = "SELECT u.id\nFROM users u\nJOIN (SELECT * FROM orders) o ON o.user_id = u.id -- :flag @x\nWHERE created_at > '2024'";
    assert!(
        error(derived_star, Dialect::PostgreSql).contains("may belong to a subquery selecting `*`")
    );
    let derived_star_qualified = "SELECT u.id\nFROM users u\nJOIN (SELECT * FROM orders) o ON o.user_id = u.id -- :flag @x\nWHERE u.email <> ''";
    assert!(parse(derived_star_qualified, Dialect::PostgreSql).is_ok());

    let two_schemas = Catalog {
        tables: vec![
            table("public", "orders", &["id", "user_id", "created_at"]),
            table("sales", "orders", &["id", "user_id", "region"]),
            table("public", "users", &["id", "email"]),
        ],
    };
    let unqualified = "SELECT u.id\nFROM users u\nJOIN orders o ON o.user_id = u.id -- :flag @x";
    let error = crate::dynfilter::strict::parse(
        unqualified,
        &params(&[]),
        Dialect::PostgreSql,
        &two_schemas,
    )
    .unwrap_err();
    assert!(
        error.contains("gated JOIN table `orders` on line 3 exists in schemas `public`, `sales`"),
        "{error}"
    );
    for (sql, expected) in [
        (
            "SELECT u.id\nFROM users u\nJOIN sales.orders o ON o.user_id = u.id -- :flag @x\nWHERE region = 'eu'",
            Some("column `region` of `o`"),
        ),
        (
            "SELECT u.id\nFROM users u\nJOIN sales.orders o ON o.user_id = u.id -- :flag @x\nWHERE created_at > '2024'",
            None,
        ),
        (
            "SELECT u.id\nFROM users u, orders\nJOIN sales.orders o ON o.user_id = u.id -- :flag @x\nWHERE region = 'eu'",
            Some(
                "may belong to table `orders`, which exists with different columns in schemas `public`, `sales`",
            ),
        ),
    ] {
        let result =
            crate::dynfilter::strict::parse(sql, &params(&[]), Dialect::PostgreSql, &two_schemas);
        match expected {
            Some(expected) => {
                let error = result.unwrap_err();
                assert!(error.contains(expected), "{sql}\n{error}");
            }
            None => assert!(result.is_ok(), "{sql}: {result:?}"),
        }
    }
}

#[test]
fn checks_gate_sets_regardless_of_order_and_extras() {
    let extra = "SELECT u.id\nFROM users u\nJOIN orders o ON o.user_id = u.id -- :flag @x\nWHERE o.id > 0 -- :flag @y @x";
    assert!(parse(extra, Dialect::PostgreSql).is_ok());
    let inherited = "SELECT u.id\nFROM users u\nJOIN orders o ON o.user_id = u.id -- :flag @x @y\nWHERE\n  u.id > 0\n  AND EXISTS ( -- :flag @y\n    SELECT 1 FROM users v WHERE v.id = u.id AND (v.id = o.user_id) -- :flag @x\n  )";
    assert!(parse(inherited, Dialect::PostgreSql).is_ok());
    let missing = "SELECT u.id\nFROM users u\nJOIN orders o ON o.user_id = u.id -- :flag @x @y\nWHERE\n  u.id > 0\n  AND EXISTS ( -- :flag @y\n    SELECT 1 FROM users v WHERE v.id = u.id AND (v.id = o.user_id)\n  )";
    assert!(error(missing, Dialect::PostgreSql).contains("alias `o` is referenced on line 7"));
    let later_join = "SELECT u.id\nFROM users u\nJOIN orders o ON o.user_id = u.id -- :flag @x\nJOIN users v ON v.id = o.user_id -- :flag @x @y\nWHERE v.id > 0 -- :flag @y @x";
    assert!(parse(later_join, Dialect::PostgreSql).is_ok());
}

#[test]
fn locates_derived_table_and_lateral_joins() {
    let inline = "SELECT u.id\nFROM users u\nJOIN (SELECT user_id FROM orders) o ON o.user_id = u.id -- :flag @x\nWHERE u.id > 0";
    let (off, on) = states(inline, Dialect::PostgreSql, &catalog());
    assert_eq!(off, "SELECT u.id\nFROM users u\nWHERE u.id > 0");
    assert_eq!(
        on,
        "SELECT u.id\nFROM users u\nJOIN (SELECT user_id FROM orders) o ON o.user_id = u.id\nWHERE u.id > 0"
    );
    let standalone = "SELECT u.id\nFROM users u\n-- :flag @x\nLEFT JOIN (\n  SELECT user_id\n  FROM orders\n) o\n  ON o.user_id = u.id\nWHERE u.id > 0";
    let (off, on) = states(standalone, Dialect::PostgreSql, &catalog());
    assert_eq!(off, "SELECT u.id\nFROM users u\nWHERE u.id > 0");
    assert_eq!(
        on,
        "SELECT u.id\nFROM users u\nLEFT JOIN (\n  SELECT user_id\n  FROM orders\n) o\n  ON o.user_id = u.id\nWHERE u.id > 0"
    );
    let lateral = "SELECT u.id\nFROM users u\nJOIN LATERAL (SELECT o.created_at FROM orders o WHERE o.user_id = u.id) l ON true -- :flag @x\nWHERE u.id > 0";
    let (off, on) = states(lateral, Dialect::PostgreSql, &catalog());
    assert_eq!(off, "SELECT u.id\nFROM users u\nWHERE u.id > 0");
    assert_eq!(
        on,
        "SELECT u.id\nFROM users u\nJOIN LATERAL (SELECT o.created_at FROM orders o WHERE o.user_id = u.id) l ON true\nWHERE u.id > 0"
    );
    let lateral_dependency = "SELECT u.id\nFROM users u\nJOIN orders o ON o.user_id = u.id -- :flag @x\nJOIN LATERAL (SELECT o.created_at AS c) l ON true\nWHERE u.id > 0";
    assert!(
        error(lateral_dependency, Dialect::PostgreSql)
            .contains("alias `o` is referenced on line 4")
    );
    let nested = "SELECT u.id\nFROM users u\nJOIN (orders o JOIN users v ON v.id = o.user_id) ON o.user_id = u.id -- :flag @x";
    assert!(error(nested, Dialect::PostgreSql).contains("gates a JOIN of a parenthesized join"));
    let unaliased = "SELECT u.id\nFROM users u\nJOIN (SELECT user_id FROM orders) ON user_id = u.id -- :flag @x";
    assert!(
        error(unaliased, Dialect::Sqlite)
            .contains("gates a JOIN of a derived table without an alias")
    );
}

#[test]
fn gates_joins_inside_set_operation_sides() {
    let sql = "SELECT u.id\nFROM users u\nJOIN orders o ON o.user_id = u.id -- :flag @x\nWHERE u.id > 0\nUNION ALL\nSELECT v.id\nFROM users v\nWHERE v.email <> ''\nORDER BY id";
    let (off, on) = states(sql, Dialect::PostgreSql, &catalog());
    assert_eq!(
        off,
        "SELECT u.id\nFROM users u\nWHERE u.id > 0\nUNION ALL\nSELECT v.id\nFROM users v\nWHERE v.email <> ''\nORDER BY id"
    );
    assert_eq!(
        on,
        "SELECT u.id\nFROM users u\nJOIN orders o ON o.user_id = u.id\nWHERE u.id > 0\nUNION ALL\nSELECT v.id\nFROM users v\nWHERE v.email <> ''\nORDER BY id"
    );
    let leak = "SELECT u.id\nFROM users u\nJOIN orders o ON o.user_id = u.id -- :flag @x\nUNION ALL\nSELECT v.id\nFROM users v\nWHERE o.id > 0";
    assert!(
        parse(leak, Dialect::PostgreSql).is_ok(),
        "a sibling side cannot see `o`"
    );
    let leak = "SELECT u.id\nFROM users u\nJOIN orders o ON o.user_id = u.id -- :flag @x\nUNION ALL\nSELECT v.id\nFROM users v\nWHERE created_at > ''";
    assert!(parse(leak, Dialect::PostgreSql).is_ok());
}

#[test]
fn resolves_output_aliases_only_as_whole_terms() {
    let shadowed = "SELECT u.id, max(u.id) AS created_at\nFROM users u\nJOIN orders o ON o.user_id = u.id -- :flag @x\nGROUP BY u.id, created_at";
    assert!(
        error(shadowed, Dialect::PostgreSql)
            .contains("column `created_at` of `o` is referenced unqualified on line 4")
    );
    let order_by_alias = "SELECT u.id, u.email AS created_at\nFROM users u\nJOIN orders o ON o.user_id = u.id -- :flag @x\nORDER BY created_at";
    for dialect in [Dialect::PostgreSql, Dialect::MySql, Dialect::Sqlite] {
        assert!(parse(order_by_alias, dialect).is_ok(), "{dialect:?}");
        for term in [
            "coalesce(created_at, '')",
            "created_at || ''",
            "(created_at)",
        ] {
            let sql = format!(
                "SELECT u.id, u.email AS created_at\nFROM users u\nJOIN orders o ON o.user_id = u.id -- :flag @x\nORDER BY {term}"
            );
            assert!(
                error(&sql, dialect)
                    .contains("column `created_at` of `o` is referenced unqualified on line 4"),
                "{dialect:?}: {term}"
            );
        }
    }
    let gated = "SELECT u.id, u.email AS created_at\nFROM users u\nJOIN orders o ON o.user_id = u.id -- :flag @x\nORDER BY\n  coalesce(created_at, '') DESC, -- :flag @x\n  created_at ASC";
    assert!(parse(gated, Dialect::PostgreSql).is_ok());

    // `audits` is not in the catalog, so `total` may be one of its columns.
    let having = "SELECT u.id, count(*) AS total\nFROM users u\nJOIN audits a ON a.user_id = u.id -- :flag @x\nGROUP BY u.id\nHAVING total > 1";
    assert!(error(having, Dialect::PostgreSql).contains("may belong to table `audits`"));
    assert!(parse(having, Dialect::Sqlite).is_ok());
    assert!(parse(having, Dialect::MySql).is_ok());
    let having_in_function = "SELECT u.id, count(*) AS total\nFROM users u\nJOIN audits a ON a.user_id = u.id -- :flag @x\nGROUP BY u.id\nHAVING coalesce(total, 0) > 1";
    assert!(error(having_in_function, Dialect::Sqlite).contains("may belong to table `audits`"));
    let having_column = "SELECT u.id, count(*) AS created_at\nFROM users u\nJOIN orders o ON o.user_id = u.id -- :flag @x\nGROUP BY u.id\nHAVING created_at > 1";
    for dialect in [Dialect::PostgreSql, Dialect::MySql, Dialect::Sqlite] {
        assert!(
            error(having_column, dialect).contains("column `created_at` of `o`"),
            "{dialect:?}"
        );
    }
}

#[test]
fn visits_distinct_on_limit_offset_and_lock_targets() {
    for (sql, expected) in [
        (
            "SELECT DISTINCT ON (o.created_at) u.id\nFROM users u\nJOIN orders o ON o.user_id = u.id -- :flag @x\n;",
            "alias `o` is referenced on line 1",
        ),
        (
            "SELECT DISTINCT ON (created_at) u.id\nFROM users u\nJOIN orders o ON o.user_id = u.id -- :flag @x\n;",
            "column `created_at` of `o` is referenced unqualified on line 1",
        ),
        (
            "SELECT u.id\nFROM users u\nJOIN orders o ON o.user_id = u.id -- :flag @x\nWHERE EXISTS (SELECT 1 FROM users i LIMIT o.id)",
            "alias `o` is referenced on line 4",
        ),
        (
            "SELECT u.id\nFROM users u\nJOIN orders o ON o.user_id = u.id -- :flag @x\nWHERE EXISTS (SELECT 1 FROM users i OFFSET o.id)",
            "alias `o` is referenced on line 4",
        ),
        (
            "SELECT u.id\nFROM users u\nJOIN orders o ON o.user_id = u.id -- :flag @x\nWHERE EXISTS (SELECT 1 FROM users i LIMIT (SELECT count(*) FROM users v WHERE v.id = o.user_id))",
            "alias `o` is referenced on line 4",
        ),
        (
            "SELECT u.id\nFROM users u\nJOIN orders o ON o.user_id = u.id -- :flag @x\nFOR UPDATE OF o",
            "alias `o` is referenced on line 4",
        ),
        (
            "SELECT u.id\nFROM users u\nJOIN orders o ON o.user_id = u.id -- :flag @x\nUNION ALL\nSELECT v.id FROM users v\nLIMIT (SELECT max(o.id) FROM orders o)",
            "",
        ),
    ] {
        let result = parse(sql, Dialect::PostgreSql);
        match expected {
            "" => assert!(result.is_ok(), "{sql}: {result:?}"),
            expected => {
                let error = result.expect_err(sql);
                assert!(error.contains(expected), "{sql}\n{error}");
            }
        }
    }
    for sql in [
        "SELECT DISTINCT ON (u.id) u.id\nFROM users u\nJOIN orders o ON o.user_id = u.id -- :flag @x\nWHERE u.id > 0\nFOR UPDATE OF u",
        "SELECT u.id\nFROM users u\nJOIN orders o ON o.user_id = u.id -- :flag @x\nWHERE\n  u.id > 0\n  AND EXISTS ( -- :flag @x\n    SELECT 1 FROM users i LIMIT o.id\n  )",
        "SELECT u.id\nFROM users u\nJOIN orders o ON o.user_id = u.id -- :flag @x\nWHERE\n  u.id > 0\n  AND EXISTS ( -- :flag @x\n    SELECT 1 FROM users i OFFSET o.id\n  )",
    ] {
        assert!(parse(sql, Dialect::PostgreSql).is_ok(), "{sql}");
    }
}

#[test]
fn applies_column_alias_lists_as_a_prefix() {
    for (sql, expected) in [
        (
            "SELECT u.id\nFROM users u\nJOIN orders o(oid, uid, stamp) ON o.uid = u.id -- :flag @x\nWHERE stamp > '2024'",
            "column `stamp` of `o` is referenced unqualified on line 4",
        ),
        (
            "SELECT u.id\nFROM users u\nJOIN orders o(oid) ON o.oid = u.id -- :flag @x\nWHERE created_at > '2024'",
            "column `created_at` of `o` is referenced unqualified on line 4",
        ),
        (
            "SELECT u.id\nFROM users u\nJOIN (SELECT user_id, created_at FROM orders) o(uid)\n  ON o.uid = u.id -- :flag @x\nWHERE created_at > '2024'",
            "column `created_at` of `o` is referenced unqualified on line 5",
        ),
        (
            "SELECT u.id\nFROM users u\nJOIN (SELECT user_id, created_at FROM orders) o(uid, stamp)\n  ON o.uid = u.id -- :flag @x\nWHERE stamp > '2024'",
            "column `stamp` of `o` is referenced unqualified on line 5",
        ),
        (
            "WITH w(a, b) AS (SELECT id, email FROM users)\nSELECT u.id\nFROM users u\nJOIN w o ON o.a = u.id -- :flag @x\nWHERE b = ''",
            "column `b` of `o` is referenced unqualified on line 5",
        ),
        (
            "SELECT u.id\nFROM users u\nJOIN orders o(\"Stamp\") ON o.\"Stamp\" = u.id -- :flag @x\nWHERE \"Stamp\" > '2024'",
            "column `Stamp` of `o` is referenced unqualified on line 4",
        ),
        (
            "SELECT u.id\nFROM users u\nJOIN (SELECT * FROM orders) o(oid)\n  ON o.oid = u.id -- :flag @x\nWHERE oid > 0",
            "column `oid` of `o` is referenced unqualified on line 5",
        ),
        (
            "SELECT u.id\nFROM users u\nJOIN (SELECT * FROM orders) o(oid)\n  ON o.oid = u.id -- :flag @x\nWHERE created_at > ''",
            "may belong to a subquery selecting `*`",
        ),
    ] {
        let error = error(sql, Dialect::PostgreSql);
        assert!(error.contains(expected), "{sql}\n{error}");
    }
    for sql in [
        "SELECT uid\nFROM users u(uid, mail)\nJOIN orders o(oid, uid2, stamp) ON o.uid2 = uid -- :flag @x\nWHERE mail <> ''",
        "SELECT u.id\nFROM users u\nJOIN orders o(oid) ON o.oid = u.id -- :flag @x\nWHERE\n  u.id > 0\n  AND created_at > '2024' -- :flag @x",
        "SELECT u.id\nFROM users u\nJOIN (SELECT user_id, created_at FROM orders) o(uid)\n  ON o.uid = u.id -- :flag @x\nWHERE\n  u.id > 0\n  AND created_at > '2024' -- :flag @x",
        "SELECT u.id\nFROM users u\nJOIN orders o(oid, uid, stamp) ON o.uid = u.id -- :flag @x\nWHERE u.id IN (SELECT user_id FROM orders x(id, user_id, stamp) WHERE stamp > '2024')",
    ] {
        assert!(parse(sql, Dialect::PostgreSql).is_ok(), "{sql}");
    }
    let (off, on) = states(
        "SELECT u.id\nFROM users u\nJOIN orders o(oid, uid, stamp) ON o.uid = u.id -- :flag @x\nWHERE\n  u.id > 0\n  AND stamp > '2024' -- :flag @x",
        Dialect::PostgreSql,
        &catalog(),
    );
    assert_eq!(off, "SELECT u.id\nFROM users u\nWHERE\n  u.id > 0");
    assert_eq!(
        on,
        "SELECT u.id\nFROM users u\nJOIN orders o(oid, uid, stamp) ON o.uid = u.id\nWHERE\n  u.id > 0\n  AND stamp > '2024'"
    );
}

#[test]
fn binds_columns_before_whole_row_references() {
    let derived = "SELECT u.id\nFROM users u\nJOIN (SELECT user_id, created_at AS u FROM orders) o\n  ON o.user_id = u.id -- :flag @x\n";
    assert!(
        error(&format!("{derived}WHERE u > '2024'"), Dialect::PostgreSql)
            .contains("column `u` of `o` is referenced unqualified on line 5")
    );
    assert!(
        parse(
            &format!("{derived}WHERE\n  u.id > 0\n  AND u > '2024' -- :flag @x"),
            Dialect::PostgreSql
        )
        .is_ok()
    );
    let same_name = "SELECT u.id\nFROM users u\nJOIN (SELECT user_id, 1 AS o FROM orders) o\n  ON o.user_id = u.id -- :flag @x\n";
    assert!(
        error(&format!("{same_name}WHERE o > 0"), Dialect::PostgreSql)
            .contains("column `o` of `o` is referenced unqualified on line 5")
    );
    // PostgreSQL also binds the column here, so a whole-row use needs a name no column has.
    assert!(
        error(
            &format!("{same_name}WHERE row_to_json(o) IS NOT NULL"),
            Dialect::PostgreSql
        )
        .contains("column `o` of `o` is referenced unqualified on line 5")
    );
    let inner_shadow =
        format!("{derived}WHERE EXISTS (SELECT 1 FROM (SELECT 1 AS u) t WHERE u > 0)");
    assert!(parse(&inner_shadow, Dialect::PostgreSql).is_ok());
    let outer_column = format!("{derived}WHERE EXISTS (SELECT 1 FROM users x WHERE u > '')");
    assert!(
        error(&outer_column, Dialect::PostgreSql)
            .contains("column `u` of `o` is referenced unqualified on line 5")
    );
    let whole_row = format!("SELECT u.id, row_to_json(o)\n{}", &JOIN[12..]);
    assert!(error(&whole_row, Dialect::PostgreSql).contains("alias `o` is referenced on line 1"));
}

#[test]
fn resolves_using_columns_against_the_left_side() {
    let gated = format!("{JOIN}JOIN users v USING (created_at)");
    assert!(
        error(&gated, Dialect::PostgreSql)
            .contains("column `created_at` of `o` is referenced unqualified on line 4")
    );
    let ungated = format!("{JOIN}JOIN users v USING (email)");
    assert!(parse(&ungated, Dialect::PostgreSql).is_ok());
}

#[test]
fn queries_without_gated_joins_skip_reference_resolution() {
    let two_schemas = Catalog {
        tables: vec![
            table("public", "users", &["id", "email"]),
            table("other", "users", &["id"]),
        ],
    };
    for sql in [
        "SELECT id FROM users\nWHERE email <> '' -- :flag @x\nORDER BY id",
        "SELECT id FROM users\nWHERE email <> ''\nORDER BY -- :switch @sort id_asc id_desc default=id_asc\n  id ASC, -- :case @id_asc\n  id DESC -- :case @id_desc\n;",
        "SELECT u.id FROM users u\nJOIN users v ON v.id = u.id\nWHERE email <> '' -- :flag @x",
    ] {
        let result =
            crate::dynfilter::strict::parse(sql, &params(&[]), Dialect::PostgreSql, &two_schemas);
        assert!(result.is_ok(), "{sql}: {result:?}");
    }
}

#[test]
fn keeps_sqlite_fts_dependencies_gated() {
    let gated = "SELECT a.id\nFROM assets a\nJOIN assets_fts ON assets_fts.rowid = a.id AND assets_fts MATCH 'x' -- :flag @fts\nWHERE a.deleted = 0\nORDER BY\n  bm25(assets_fts) ASC, -- :flag @fts\n  a.id ASC";
    assert!(parse_uncatalogued(gated, &params(&[]), Dialect::Sqlite).is_ok());
    for (sql, expected) in [
        (
            "SELECT a.id\nFROM assets a\nJOIN assets_fts ON assets_fts.rowid = a.id AND assets_fts MATCH 'x' -- :flag @fts\nWHERE a.deleted = 0\nORDER BY bm25(assets_fts) ASC",
            "alias `assets_fts` is referenced on line 5",
        ),
        (
            "SELECT a.id\nFROM assets a\nJOIN assets_fts ON assets_fts.rowid = a.id -- :flag @fts\nWHERE assets_fts MATCH 'x'",
            "alias `assets_fts` is referenced on line 4",
        ),
    ] {
        let error = parse_uncatalogued(sql, &params(&[]), Dialect::Sqlite).unwrap_err();
        assert!(error.contains(expected), "{sql}\n{error}");
    }
}

//! Typed SQLx PostgreSQL warm-ups: bind order, overrides, and the shared-text bind-type check.

use super::{Fixture, Mode, matrix, region};
use crate::{
    db_crates::{
        DbCrate, Sqlx,
        test_support::{column, query},
    },
    plugin,
    query::RsType,
};

fn typed_fixture() -> Fixture {
    let mut nick = column("nick", false);
    nick.not_null = false;
    let mut status = column("status", false);
    status.not_null = false;
    let mut tags = column("tags", false);
    tags.is_array = true;
    tags.array_dims = 1;
    tags.r#type = Some(crate::db_crates::test_support::identifier("text"));
    let mut fixture = Fixture::new(
        DbCrate::Sqlx(Sqlx::Postgres),
        vec![
            query(
                "A",
                ":many",
                "SELECT id FROM users\nWHERE\n  email = $1 -- :if @email\n  AND id = $2 -- :if @id\n  AND tags && $3::text[] -- :if @tags",
                vec![column("id", false)],
                vec![
                    (1, column("email", false)),
                    (2, column("id", false)),
                    (3, tags),
                ],
            ),
            query(
                "B",
                ":many",
                "SELECT id FROM users\nWHERE\n  nick = $1 -- :if @nick\n  AND alt = $1 -- :if @nick\n  AND status = $2",
                vec![column("id", false)],
                vec![(1, nick), (2, status)],
            ),
        ],
    );
    let mut email = column("email", false);
    email.r#type = Some(crate::db_crates::test_support::identifier("text"));
    fixture.queries[0].params[0].column = Some(email.clone());
    fixture.queries[1].params[0].column.as_mut().unwrap().r#type = email.r#type.clone();
    fixture.type_map.insert_column_type(
        ".id",
        RsType::new(syn::parse_str("i64").unwrap(), None, true),
    );
    fixture
}

#[test]
fn sqlx_postgres_warmups_prepare_with_rust_bind_types() {
    let fixture = typed_fixture();
    let tokens = fixture.generate(Mode::On).unwrap();
    let warmup = region(&tokens, "pub async fn prepare_a (", "Ok (())");
    let info = |typ: &str| format!("< {typ} as sqlx :: Type < sqlx :: Postgres >> :: type_info ()");
    // email, id, tags as a 3-bit counter: 0 = none, 2 = id only, 5 = email + tags, 7 = all.
    assert!(
        warmup.contains("prepare_with (& mut * conn , A_VARIANTS [0] , & []) . await ?"),
        "{warmup}"
    );
    assert!(
        warmup.contains("\"dynfilters.prepared warm-up needs statement_cache_capacity > 0"),
        "{warmup}"
    );
    assert!(
        warmup.contains(&format!("A_VARIANTS [2] , & [{} ,]", info("i64"))),
        "{warmup}"
    );
    assert!(
        warmup.contains(&format!(
            "A_VARIANTS [5] , & [{} , {} ,]",
            info("& str"),
            info("& [String]")
        )),
        "{warmup}"
    );
    assert!(
        warmup.contains(&format!(
            "A_VARIANTS [7] , & [{} , {} , {} ,]",
            info("& str"),
            info("i64"),
            info("& [String]")
        )),
        "{warmup}"
    );
    assert_eq!(warmup.matches("prepare_with").count(), 8);
    // The disabled-cache guard runs once, right after the first prepare.
    assert_eq!(
        warmup
            .matches("cached_statements_size (& * conn) == 0")
            .count(),
        1
    );
    assert!(
        warmup.contains(
            "A_VARIANTS [0] , & []) . await ? ; if sqlx :: Connection :: cached_statements_size"
        ),
        "{warmup}"
    );

    let warmup = region(&tokens, "pub async fn prepare_b (", "Ok (())");
    assert!(
        warmup.contains(&format!(
            "B_VARIANTS [0] , & [{} ,]",
            info("Option < i32 >")
        )),
        "{warmup}"
    );
    assert!(
        warmup.contains(&format!(
            "B_VARIANTS [1] , & [{} , {} ,]",
            info("& str"),
            info("Option < i32 >")
        )),
        "{warmup}"
    );
    assert!(
        tokens.contains(
            "pub async fn prepare_dynfilter_variants (conn : & mut sqlx :: PgConnection)"
        )
    );

    for backend in [DbCrate::Sqlx(Sqlx::MySql), DbCrate::Sqlx(Sqlx::Sqlite)] {
        let tokens = matrix(backend).generate(Mode::On).unwrap();
        let warmup = region(&tokens, "pub async fn prepare_search_users (", "Ok (())");
        assert!(warmup.contains("for sql in SEARCH_USERS_VARIANTS { sqlx :: Executor :: prepare (& mut * conn , sql) . await ? ; }"), "{warmup}");
        assert!(!tokens.contains("prepare_with"));
    }
}

#[test]
fn sqlx_postgres_rejects_one_sql_text_with_two_bind_type_vectors() {
    let mut code = column("code", false);
    code.r#type = Some(crate::db_crates::test_support::identifier("bigint"));
    let mut name_int = column("name", false);
    name_int.r#type = Some(crate::db_crates::test_support::identifier("bigint"));
    let mut name_text = column("name", false);
    name_text.r#type = Some(crate::db_crates::test_support::identifier("text"));
    let mut fixture = Fixture::new(
        DbCrate::Sqlx(Sqlx::Postgres),
        vec![query(
            "Q",
            ":many",
            "SELECT id FROM users\nWHERE\n  id > 0\n  AND (\n    name = $1 -- :if @name\n    OR name = $2 -- :if @code\n  )",
            vec![column("id", false)],
            vec![(1, name_text.clone()), (2, code.clone())],
        )],
    );
    let error = fixture.generate(Mode::On).unwrap_err().to_string();
    // The `code`-only state is enumerated first, so it holds the representative bind plan.
    assert_eq!(
        error,
        "Query `Q`: SQL text `SELECT id FROM users WHERE   id > 0   AND (      FALSE     OR name = $1   )` is bound as [< i64 as sqlx :: Type < sqlx :: Postgres >> :: type_info ()] by cached dynamic query `Q` but as [< & str as sqlx :: Type < sqlx :: Postgres >> :: type_info ()] by cached dynamic query `Q`; one connection cache entry would serve both, so add every dynamic query sharing the text to dynfilters.prepared_skip: `Q`"
    );
    // Skipping the only source of the text leaves nothing that could cache it.
    fixture.dynfilters.prepared_skip = vec!["Q".to_string()];
    fixture.generate(Mode::On).unwrap();

    let dynamic = |name: &str, param: plugin::Column| {
        query(
            name,
            ":many",
            "SELECT id FROM users\nWHERE\n  name = $1 -- :if @name",
            vec![column("id", false)],
            vec![(1, param)],
        )
    };
    let mut fixture = Fixture::new(
        DbCrate::Sqlx(Sqlx::Postgres),
        vec![
            dynamic("First", name_text.clone()),
            dynamic("Second", name_int.clone()),
        ],
    );
    let expect_conflict = |fixture: &Fixture, fix: &str| {
        let error = fixture.generate(Mode::On).unwrap_err().to_string();
        assert!(
            error.starts_with("Query `First`: SQL text `SELECT id FROM users WHERE   name = $1` is bound as [< & str as"),
            "{error}"
        );
        assert!(
            error.contains("by cached dynamic query `First` but as [< i64 as"),
            "{error}"
        );
        assert!(error.ends_with(fix), "{error}");
    };
    expect_conflict(&fixture, "dynfilters.prepared_skip: `First`, `Second`");
    // `persistent(false)` still reads a cached entry, so skipping one side is not enough.
    fixture.dynfilters.prepared_skip = vec!["Second".to_string()];
    let error = fixture.generate(Mode::On).unwrap_err().to_string();
    assert!(
        error.contains("by cache-skipped dynamic query `Second`"),
        "{error}"
    );
    fixture.dynfilters.prepared_skip = vec!["First".to_string(), "Second".to_string()];
    fixture.generate(Mode::On).unwrap();

    // A static query always caches its text and cannot be skipped.
    let static_query = query(
        "Fixed",
        ":many",
        "SELECT id FROM users\nWHERE\n  name = $1",
        vec![column("id", false)],
        vec![(1, name_int)],
    );
    let mut fixture = Fixture::new(
        DbCrate::Sqlx(Sqlx::Postgres),
        vec![dynamic("First", name_text), static_query],
    );
    let error = fixture.generate(Mode::On).unwrap_err().to_string();
    assert!(error.contains("by static query `Fixed`"), "{error}");
    assert!(error.ends_with("a static query always caches its text, so change the SQL of `Fixed` or of the dynamic query"), "{error}");
    fixture.dynfilters.prepared_skip = vec!["First".to_string()];
    let error = fixture.generate(Mode::On).unwrap_err().to_string();
    assert!(error.contains("by static query `Fixed`"), "{error}");
    // Matching types are fine.
    fixture.dynfilters.prepared_skip.clear();
    fixture.queries[1].params[0].column.as_mut().unwrap().r#type =
        Some(crate::db_crates::test_support::identifier("text"));
    fixture.generate(Mode::On).unwrap();
    // sqlx reports one `type_info` for `T` and `Option<T>`, so a nullable static parameter
    // binds the same cache entry as the conditional one that unwraps its `Option`.
    fixture.queries[1].params[0]
        .column
        .as_mut()
        .unwrap()
        .not_null = false;
    fixture.generate(Mode::On).unwrap();
}

#[test]
fn sqlx_postgres_warmups_follow_runtime_bind_order_and_accept_same_class_alternates() {
    let mut code = column("code", false);
    code.r#type = Some(crate::db_crates::test_support::identifier("bigint"));
    let mut email = column("email", false);
    email.r#type = Some(crate::db_crates::test_support::identifier("text"));
    // `$2` appears before `$1` and sqlc reports the parameters in reverse order.
    let mut fixture = Fixture::new(
        DbCrate::Sqlx(Sqlx::Postgres),
        vec![query(
            "Reordered",
            ":many",
            "SELECT id FROM users\nWHERE\n  code = $2 -- :if @code\n  AND email = $1 -- :if @email",
            vec![column("id", false)],
            vec![(2, code), (1, email.clone())],
        )],
    );
    let tokens = fixture.generate(Mode::On).unwrap();
    let info = |typ: &str| format!("< {typ} as sqlx :: Type < sqlx :: Postgres >> :: type_info ()");
    let warmup = region(&tokens, "pub async fn prepare_reordered (", "Ok (())");
    // Counter order is email (param 1) then code (param 2): index 3 has both active, and the
    // runtime numbers `code` first because it comes first in the text.
    assert!(
        warmup.contains(&format!(
            "REORDERED_VARIANTS [3] , & [{} , {} ,]",
            info("i64"),
            info("& str")
        )),
        "{warmup}"
    );
    assert!(
        warmup.contains(&format!("REORDERED_VARIANTS [1] , & [{} ,]", info("i64"))),
        "{warmup}"
    );
    assert!(
        warmup.contains(&format!("REORDERED_VARIANTS [2] , & [{} ,]", info("& str"))),
        "{warmup}"
    );
    let constant = region(&tokens, "pub const REORDERED_VARIANTS", "] ;");
    assert!(
        constant.contains("code = $1\n  AND email = $2"),
        "{constant}"
    );

    // Two same-typed operands collapse to one text with two source parameters; the shared
    // prepared statement is valid for both and generation accepts it.
    let mut alt = column("alt", false);
    alt.r#type = Some(crate::db_crates::test_support::identifier("text"));
    fixture.queries = vec![query(
        "Either",
        ":many",
        "SELECT id FROM users\nWHERE\n  id > 0\n  AND (\n    email = $1 -- :if @email\n    OR email = $2 -- :if @alt\n  )",
        vec![column("id", false)],
        vec![(1, email), (2, alt)],
    )];
    let tokens = fixture.generate(Mode::On).unwrap();
    assert!(
        tokens.contains("pub const DYNFILTER_VARIANT_COUNT : usize = 3"),
        "{tokens}"
    );

    // An owned non-Copy override binds by reference and is prepared as its owned type; both
    // operands must take it, or the shared text would bind two types.
    for name in [".email", ".alt"] {
        fixture.type_map.insert_column_type(
            name,
            RsType::new(syn::parse_str("String").unwrap(), None, false),
        );
    }
    let tokens = fixture.generate(Mode::On).unwrap();
    assert!(
        tokens.contains(
            "dynfilter :: Bind :: Arg (0usize) => q . bind (params . email . as_ref () . unwrap ())"
        ),
        "{tokens}"
    );
    // Both single-operand states render one text, so index 1 carries the one-bind plan.
    let warmup = region(&tokens, "pub async fn prepare_either (", "Ok (())");
    assert!(
        warmup.contains(&format!("EITHER_VARIANTS [1] , & [{} ,]", info("String"))),
        "{warmup}"
    );
}

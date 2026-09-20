use super::Dialect;
use super::variants::{Options, Variant, expand, expand_all};
use crate::db_crates::{
    DbCrate, Sqlx,
    test_support::{column, query},
};
use crate::dynfilter_runtime::{Dialect as RuntimeDialect, Placeholders};
use crate::query::Query;

pub(crate) const SEARCH_USERS: &str = "SELECT id, email, phone\nFROM users\nWHERE\n  email = $1 -- :if @email\n  AND phone = $2 -- :if @phone\n  AND EXISTS ( -- :flag @has_orders\n    SELECT 1 FROM orders WHERE orders.user_id = users.id\n      AND orders.created_at >= $3 -- :if @orders_since\n  )\n  AND id = ANY($4::bigint[]) -- :if @ids\nORDER BY -- :switch @sort id_asc id_desc shortest_email default=id_asc\n  id ASC, -- :case @id_asc\n  id DESC, -- :case @id_desc\n  LENGTH(email) ASC, id DESC -- :case @shortest_email\nLIMIT $5";
pub(crate) const SEARCH_USERS_PARAMS: &[(&str, bool)] = &[
    ("email", false),
    ("phone", false),
    ("orders_since", false),
    ("ids", false),
    ("row_limit", false),
];

pub(crate) fn dynamic(
    db_crate: DbCrate,
    dialect: Dialect,
    name: &str,
    sql: &str,
    params: &[(&str, bool)],
) -> Query {
    let params = params
        .iter()
        .enumerate()
        .map(|(index, (name, slice))| (index as i32 + 1, column(name, *slice)))
        .collect();
    let plugin = query(name, ":many", sql, vec![column("id", false)], params);
    Query::parse(
        &db_crate.db_type_map(),
        &plugin,
        dialect,
        &Default::default(),
        false,
    )
    .unwrap()
}

fn postgres(name: &str, sql: &str, params: &[(&str, bool)]) -> Query {
    dynamic(DbCrate::default(), Dialect::PostgreSql, name, sql, params)
}

#[test]
fn dedups_collapsed_states_and_keeps_switches_exclusive() {
    let query = postgres("SearchUsers", SEARCH_USERS, SEARCH_USERS_PARAMS);
    let variants = expand(
        &query,
        Placeholders::Numbered,
        RuntimeDialect::Postgres,
        1024,
        None,
    )
    .unwrap()
    .variants;

    // 2^5 boolean controls x 3 sort choices, minus the `orders_since` states that render
    // identically while `has_orders` is off.
    assert_eq!(variants.len(), 96 - 24);
    let sorts = [
        "ORDER BY\n  id ASC\nLIMIT $",
        "ORDER BY\n  id DESC\nLIMIT $",
        "ORDER BY\n  LENGTH(email) ASC,\n  id DESC\nLIMIT $",
    ];
    fn order_by(variant: &Variant) -> &str {
        &variant.sql[variant.sql.find("ORDER BY").unwrap()..]
    }
    let per_sort = sorts.map(|sort| {
        variants
            .iter()
            .filter(|variant| order_by(variant).starts_with(sort))
            .count()
    });
    assert_eq!(per_sort, [24, 24, 24]);
    assert!(variants.iter().all(|variant| !variant.sql.contains("-- :")));
    assert_eq!(
        variants[0].sql,
        "SELECT id, email, phone\nFROM users\nORDER BY\n  id ASC\nLIMIT $1"
    );

    let again = expand(
        &query,
        Placeholders::Numbered,
        RuntimeDialect::Postgres,
        1024,
        None,
    )
    .unwrap()
    .variants;
    assert!(
        variants
            .iter()
            .zip(&again)
            .all(|(a, b)| a.sql == b.sql && a.binds == b.binds)
    );
}

#[test]
fn renders_engine_placeholders_and_single_element_slices() {
    let sqlite = dynamic(
        DbCrate::Rusqlite,
        Dialect::Sqlite,
        "Q",
        "SELECT id FROM users\nWHERE\n  email = ?1 -- :if @email\n  AND id IN (/*SLICE:ids*/?2) -- :if @ids",
        &[("email", false), ("ids", true)],
    );
    let variants = expand(
        &sqlite,
        Placeholders::NumberedSqlite,
        RuntimeDialect::Sqlite,
        1024,
        None,
    )
    .unwrap()
    .variants;
    let sql = variants.iter().map(|v| v.sql.as_str()).collect::<Vec<_>>();
    assert_eq!(
        sql,
        [
            "SELECT id FROM users",
            "SELECT id FROM users\nWHERE\n  id IN ($1)",
            "SELECT id FROM users\nWHERE\n  email = $1",
            "SELECT id FROM users\nWHERE\n  email = $1\n  AND id IN ($2)",
        ]
    );
    assert!(sql.iter().all(|sql| !sql.contains("/*SLICE:")));

    let mysql = dynamic(
        DbCrate::Sqlx(Sqlx::MySql),
        Dialect::MySql,
        "Q",
        "SELECT id FROM users\nWHERE\n  email = ? -- :if @email\n  AND id IN (/*SLICE:ids*/?) -- :if @ids",
        &[("email", false), ("ids", true)],
    );
    let variants = expand(
        &mysql,
        Placeholders::Question,
        RuntimeDialect::MySql,
        1024,
        None,
    )
    .unwrap()
    .variants;
    assert_eq!(
        variants[3].sql,
        "SELECT id FROM users\nWHERE\n  email = ?\n  AND id IN (?)"
    );

    let postgres = postgres(
        "Q",
        "SELECT id FROM users\nWHERE\n  email = $1 -- :if @email\n  AND phone = $1 -- :if @email",
        &[("email", false)],
    );
    let variants = expand(
        &postgres,
        Placeholders::Numbered,
        RuntimeDialect::Postgres,
        1024,
        None,
    )
    .unwrap()
    .variants;
    assert_eq!(
        variants[1].sql,
        "SELECT id FROM users\nWHERE\n  email = $1\n  AND phone = $1"
    );
    assert_eq!(variants[1].binds.len(), 1);
}

#[test]
fn enforces_limit_and_skip_list() {
    let query = postgres("SearchUsers", SEARCH_USERS, SEARCH_USERS_PARAMS);
    let error = expand(
        &query,
        Placeholders::Numbered,
        RuntimeDialect::Postgres,
        10,
        None,
    )
    .unwrap_err()
    .to_string();
    assert_eq!(
        error,
        "Query `SearchUsers`: dynamic-filter variants exceed dynfilters.variant_limit 10; raise dynfilters.variant_limit or add the query to dynfilters.variants_skip"
    );

    let deduped = expand(
        &query,
        Placeholders::Numbered,
        RuntimeDialect::Postgres,
        1024,
        None,
    )
    .unwrap()
    .variants
    .len();
    assert_eq!(
        expand(
            &query,
            Placeholders::Numbered,
            RuntimeDialect::Postgres,
            deduped,
            None
        )
        .unwrap()
        .variants
        .len(),
        deduped
    );
    assert_eq!(
        expand(
            &query,
            Placeholders::Numbered,
            RuntimeDialect::Postgres,
            0,
            None
        )
        .unwrap_err()
        .to_string(),
        "Query `SearchUsers`: dynamic-filter variants exceed dynfilters.variant_limit 0; raise dynfilters.variant_limit or add the query to dynfilters.variants_skip"
    );

    let plain = Query::from_query(
        &DbCrate::default().db_type_map(),
        &crate::db_crates::test_support::query(
            "Plain",
            ":many",
            "SELECT id FROM users",
            vec![column("id", false)],
            Vec::new(),
        ),
    )
    .unwrap();
    let queries = [query, plain];
    let options = |skip: &'static [String]| Options {
        placeholders: Placeholders::Numbered,
        dialect: RuntimeDialect::Postgres,
        limit: 10,
        skip,
        cache_skip: &[],
        bind_classes: None,
    };
    let skipped = expand_all(&queries, &options(vec!["SearchUsers".to_string()].leak())).unwrap();
    assert!(skipped.is_empty());
    for name in ["Plain", "Missing"] {
        let error = expand_all(&queries, &options(vec![name.to_string()].leak()))
            .err()
            .expect("skip list rejected")
            .to_string();
        assert_eq!(
            error,
            format!("Query `{name}`: dynfilters.variants_skip names no dynamic-filter query")
        );
    }
}

#[test]
fn enumerates_queries_whose_only_controls_are_flags_or_switches() {
    let flag = postgres(
        "Q",
        "SELECT id FROM users\nWHERE\n  phone <> '' -- :flag @with_phone",
        &[],
    );
    let sql = |query: &Query| {
        expand(
            query,
            Placeholders::Numbered,
            RuntimeDialect::Postgres,
            1024,
            None,
        )
        .unwrap()
        .variants
        .into_iter()
        .map(|variant| variant.sql)
        .collect::<Vec<_>>()
    };
    assert_eq!(
        sql(&flag),
        [
            "SELECT id FROM users",
            "SELECT id FROM users\nWHERE\n  phone <> ''"
        ]
    );

    let switch = postgres(
        "Q",
        "SELECT id FROM users\nORDER BY -- :switch @sort asc desc default=asc\n  id ASC, -- :case @asc\n  id DESC -- :case @desc",
        &[],
    );
    assert_eq!(
        sql(&switch),
        [
            "SELECT id FROM users\nORDER BY\n  id ASC",
            "SELECT id FROM users\nORDER BY\n  id DESC"
        ]
    );
}

#[test]
fn keeps_the_base_cmd_and_skips_static_slice_queries() {
    let sqlite = |name, cmd, sql, columns, params| {
        Query::parse(
            &DbCrate::Rusqlite.db_type_map(),
            &query(name, cmd, sql, columns, params),
            Dialect::Sqlite,
            &Default::default(),
            true,
        )
        .unwrap()
    };
    let execrows = sqlite(
        "TouchUsers",
        ":execrows",
        "UPDATE users SET phone = phone\nWHERE\n  email = ?1 -- :if @email",
        Vec::new(),
        vec![(1, column("email", false))],
    );
    let one = sqlite(
        "CountUsers",
        ":one",
        "SELECT COUNT(*) AS total FROM users\nWHERE\n  email = ?1 -- :if @email",
        vec![column("total", false)],
        vec![(1, column("email", false))],
    );
    let static_slice = sqlite(
        "ByIds",
        ":many",
        "SELECT id FROM users WHERE id IN (/*SLICE:ids*/?1)",
        vec![column("id", false)],
        vec![(1, column("ids", true))],
    );
    assert!(static_slice.dynfilter().is_some());

    let queries = [execrows, one, static_slice];
    let expansions = expand_all(
        &queries,
        &Options {
            placeholders: Placeholders::NumberedSqlite,
            dialect: RuntimeDialect::Sqlite,
            limit: 1024,
            skip: &[],
            cache_skip: &[],
            bind_classes: None,
        },
    )
    .unwrap();
    assert_eq!(
        expansions
            .iter()
            .map(|expansion| (
                expansion.query.query_name.as_str(),
                expansion.variants.len()
            ))
            .collect::<Vec<_>>(),
        [("TouchUsers", 2), ("CountUsers", 2)]
    );
}

#[test]
fn caps_raw_control_states_before_rendering() {
    let flags = (0..21)
        .map(|index| format!("  AND id > {index} -- :flag @f{index}"))
        .collect::<Vec<_>>()
        .join("\n");
    let query = postgres(
        "Wide",
        &format!("SELECT id FROM users\nWHERE\n  id > 0\n{flags}"),
        &[],
    );
    let started = std::time::Instant::now();
    let error = expand(
        &query,
        Placeholders::Numbered,
        RuntimeDialect::Postgres,
        usize::MAX,
        None,
    )
    .unwrap_err()
    .to_string();
    assert_eq!(
        error,
        format!(
            "Query `Wide`: dynamic-filter control states exceed the {} raw-state cap; reduce its independent controls or add the query to dynfilters.variants_skip",
            super::variants::MAX_RAW_STATES
        )
    );
    assert!(started.elapsed().as_secs() < 5);

    let query = postgres(
        "Wide",
        &format!(
            "SELECT id FROM users\nWHERE\n  id > 0\n{}",
            flags.replace("id > ", "id >= ")
        ),
        &[],
    );
    let error = expand(
        &query,
        Placeholders::Numbered,
        RuntimeDialect::Postgres,
        3,
        None,
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("exceed the"), "{error}");
}

#[test]
fn switch_choices_enumerate_in_declaration_order() {
    let query = postgres(
        "Q",
        "SELECT id FROM users\nORDER BY -- :switch @sort newest oldest by_email default=newest\n  email ASC, -- :case @by_email\n  id ASC, -- :case @oldest\n  id DESC -- :case @newest",
        &[],
    );
    let variants = expand(
        &query,
        Placeholders::Numbered,
        RuntimeDialect::Postgres,
        1024,
        None,
    )
    .unwrap()
    .variants;
    assert_eq!(
        variants
            .iter()
            .map(|variant| variant.sql.as_str())
            .collect::<Vec<_>>(),
        [
            "SELECT id FROM users\nORDER BY\n  id DESC",
            "SELECT id FROM users\nORDER BY\n  id ASC",
            "SELECT id FROM users\nORDER BY\n  email ASC",
        ]
    );
}

#[test]
fn gated_join_off_state_drops_the_join_and_its_order_term() {
    let query = postgres(
        "SearchUsersWithOrders",
        "SELECT u.id, u.email, u.phone\nFROM users u\nJOIN orders o ON o.user_id = u.id AND o.created_at >= $1 -- :if @orders_since\nWHERE\n  u.phone <> '' -- :flag @with_phone\nORDER BY\n  o.created_at DESC, -- :if @orders_since\n  u.id ASC",
        &[("orders_since", false)],
    );
    let variants = expand(
        &query,
        Placeholders::Numbered,
        RuntimeDialect::Postgres,
        1024,
        None,
    )
    .unwrap()
    .variants;
    assert_eq!(variants.len(), 4);
    let off = &variants[0];
    assert_eq!(
        off.sql,
        "SELECT u.id, u.email, u.phone\nFROM users u\nORDER BY\n  u.id ASC"
    );
    assert!(!off.sql.contains("JOIN") && !off.sql.contains("o."));
    assert!(off.binds.is_empty());
    let on = &variants[2];
    assert!(
        on.sql
            .contains("JOIN orders o ON o.user_id = u.id AND o.created_at >= $1")
    );
    assert!(
        on.sql
            .contains("ORDER BY\n  o.created_at DESC,\n  u.id ASC")
    );
    assert_eq!(on.binds.len(), off.binds.len() + 1);
}

#[test]
fn standalone_annotation_line_before_a_join_never_reaches_a_variant() {
    let query = postgres(
        "SearchUsersByLastOrder",
        "SELECT u.id, u.email\nFROM users u\n  -- :flag @with_orders\nJOIN (\n  SELECT user_id, max(created_at) AS last_order_at\n  FROM orders\n  GROUP BY user_id\n) o ON o.user_id = u.id\nWHERE\n  u.id > 0\n  AND last_order_at >= '2024-06-01' -- :flag @with_orders\nORDER BY\n  coalesce(last_order_at, '') DESC, -- :flag @with_orders\n  u.id ASC",
        &[],
    );
    let variants = expand(
        &query,
        Placeholders::Numbered,
        RuntimeDialect::Postgres,
        1024,
        None,
    )
    .unwrap()
    .variants;
    assert_eq!(variants.len(), 2);
    assert_eq!(
        variants[0].sql,
        "SELECT u.id, u.email\nFROM users u\nWHERE\n  u.id > 0\nORDER BY\n  u.id ASC"
    );
    assert!(variants[1].sql.contains("FROM users u\nJOIN (\n"));
    assert!(variants.iter().all(|variant| !variant.sql.contains("-- :")));
}

#[test]
fn or_group_with_every_operand_gated_renders_false_when_inactive() {
    let query = postgres(
        "SearchUsersByPattern",
        "SELECT id, email, phone\nFROM users\nWHERE\n  id > 0\n  AND (\n    email LIKE $1 -- :if @email_pattern\n    OR phone LIKE $2 -- :if @phone_pattern\n  )\nORDER BY id",
        &[("email_pattern", false), ("phone_pattern", false)],
    );
    let variants = expand(
        &query,
        Placeholders::Numbered,
        RuntimeDialect::Postgres,
        1024,
        None,
    )
    .unwrap()
    .variants;
    let sql = variants.iter().map(|v| v.sql.as_str()).collect::<Vec<_>>();
    assert_eq!(sql.len(), 4);
    assert!(sql[0].contains("AND (\n\n    FALSE\n  )"), "{}", sql[0]);
    assert!(variants[0].binds.is_empty());
    assert!(
        sql[1].contains("FALSE\n    OR phone LIKE $1\n  )"),
        "{}",
        sql[1]
    );
    assert_eq!(sql[1].matches(" OR ").count(), 1);
    assert!(
        sql[2].contains("FALSE\n    OR email LIKE $1\n  )"),
        "{}",
        sql[2]
    );
    assert!(
        sql[3].contains("FALSE\n    OR email LIKE $1\n    OR phone LIKE $2\n  )"),
        "{}",
        sql[3]
    );
    assert_eq!(variants[3].binds.len(), 2);
}

#[test]
fn postgres_json_question_operator_is_not_a_placeholder() {
    let query = postgres(
        "CountUsersWithJsonKey",
        "SELECT COUNT(*) AS total\nFROM users\nWHERE\n  '{\"a\": 1}'::jsonb ? $1::text\n  AND email = $2 -- :if @email\n  AND '{\"b\": 1}'::jsonb ?| ARRAY['b', 'c']",
        &[("key", false), ("email", false)],
    );
    let variants = expand(
        &query,
        Placeholders::Numbered,
        RuntimeDialect::Postgres,
        1024,
        None,
    )
    .unwrap()
    .variants;
    assert_eq!(variants.len(), 2);
    assert_eq!(
        variants[0].sql,
        "SELECT COUNT(*) AS total\nFROM users\nWHERE\n  '{\"a\": 1}'::jsonb ? $1::text\n  AND '{\"b\": 1}'::jsonb ?| ARRAY['b', 'c']"
    );
    assert_eq!(variants[0].binds.len(), 1);
    assert!(variants[1].sql.contains("? $1::text\n  AND email = $2\n"));
    assert_eq!(variants[1].binds.len(), 2);
}

/// Many same-typed operands collapse to few texts with many bind plans: 16 controls give 17
/// texts and 65,536 states. Alternates are decided by a per-state class signature, never by
/// scanning retained plans, and only when a classifier is supplied.
#[test]
fn collapsing_states_record_alternates_by_type_class_only_when_asked() {
    let operands = (1..=16)
        .map(|n| format!("    id = ${n} -- :if @p{n}"))
        .collect::<Vec<_>>()
        .join("\n    OR ");
    let sql = format!("SELECT id FROM users\nWHERE\n  id > 0\n  AND (\n{operands}\n  )");
    let names = (1..=16).map(|n| format!("p{n}")).collect::<Vec<_>>();
    let params = names
        .iter()
        .map(|name| (name.as_str(), false))
        .collect::<Vec<_>>();
    let query = postgres("Collapsing", &sql, &params);

    let started = std::time::Instant::now();
    let plain = expand(
        &query,
        Placeholders::Numbered,
        RuntimeDialect::Postgres,
        1024,
        None,
    )
    .unwrap()
    .variants;
    let plain_elapsed = started.elapsed();
    assert_eq!(plain.len(), 17);
    assert!(plain.iter().all(|variant| variant.alternate.is_none()));

    let same_class = vec![0; 16];
    let classed = expand(
        &query,
        Placeholders::Numbered,
        RuntimeDialect::Postgres,
        1024,
        Some(&same_class),
    )
    .unwrap()
    .variants;
    assert_eq!(classed.len(), 17);
    assert!(classed.iter().all(|variant| variant.alternate.is_none()));

    // One differently typed operand: every text that can include it or another operand in
    // the same slot gets exactly one alternate, not one per state.
    let mut mixed = vec![0; 16];
    mixed[15] = 1;
    let mixed = expand(
        &query,
        Placeholders::Numbered,
        RuntimeDialect::Postgres,
        1024,
        Some(&mixed),
    )
    .unwrap()
    .variants;
    let with_alternate = mixed
        .iter()
        .filter(|variant| variant.alternate.is_some())
        .count();
    assert_eq!(
        with_alternate,
        15,
        "{:?}",
        mixed.iter().map(|v| &v.alternate).collect::<Vec<_>>()
    );
    assert!(mixed.iter().all(|variant| {
        variant
            .alternate
            .as_ref()
            .is_none_or(|binds| binds.len() == variant.binds.len())
    }));
    // Signature work is linear in binds per state; guard against a return to plan scanning.
    assert!(plain_elapsed.as_secs() < 30, "{plain_elapsed:?}");
}

/// The generated call path answers every control state from `X_STATES`, so the table must
/// hold, for each raw state, exactly what `Compiled::build` renders for it.
#[test]
fn state_table_matches_the_renderer_for_every_control_state() {
    use super::variants::{base_args, controls};
    use crate::dynfilter_runtime::compile_with_arg_order;

    let query = postgres("SearchUsers", SEARCH_USERS, SEARCH_USERS_PARAMS);
    let expanded = expand(
        &query,
        Placeholders::Numbered,
        RuntimeDialect::Postgres,
        1024,
        None,
    )
    .unwrap();
    let states = expanded.states.unwrap();
    assert!(states.windows(2).all(|pair| pair[0].gates < pair[1].gates));
    assert!(states.len() >= expanded.variants.len());

    let info = query.expect_dynfilter();
    let compiled = compile_with_arg_order(
        &info.annotated_sql,
        Placeholders::Numbered,
        RuntimeDialect::Postgres,
        query.arg_order(),
        &info.runtime_plan(),
    );
    let slots = query.arg_slots();
    let controls = controls(info, &slots);
    let total: usize = controls.iter().map(|control| control.radix).product();
    let mut args = base_args(info, &slots);
    for counter in 0..total {
        controls.iter().rev().fold(counter, |rest, control| {
            control.apply(&mut args, rest % control.radix);
            rest / control.radix
        });
        let gates = compiled.gate_state(&args).unwrap();
        let active = compiled.active_gates(&args);
        assert!(
            active
                .iter()
                .enumerate()
                .all(|(bit, on)| *on == (gates >> bit & 1 == 1)),
            "{counter}"
        );
        let state = &states[states
            .binary_search_by_key(&gates, |state| state.gates)
            .unwrap()];
        let (sql, binds) = compiled.build(&args);
        assert_eq!(expanded.variants[state.variant].sql, sql, "{counter}");
        assert_eq!(state.binds, binds, "{counter}");
    }
}

//! `dynfilters.prepared` token tests across every backend: eligibility, emitted constants and
//! warm-ups, cache-path changes, and identifier reservation. SQLx PostgreSQL typing lives in
//! `typing`.

mod typing;

use crate::{
    db_crates::{
        DbCrate, Postgres, Sqlx,
        test_support::{column, dialect, generate, parsed, query},
    },
    dynfilter::{
        DynFilters,
        variants::{Options, expand_all},
    },
    plugin,
    query::{DbTypeMap, QueryError},
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Mode {
    Off,
    On,
}

pub(super) struct Fixture {
    pub(super) backend: DbCrate,
    pub(super) type_map: DbTypeMap,
    pub(super) queries: Vec<plugin::Query>,
    pub(super) dynfilters: DynFilters,
}

impl Fixture {
    pub(super) fn new(backend: DbCrate, queries: Vec<plugin::Query>) -> Self {
        Self {
            backend,
            type_map: backend.db_type_map(),
            queries,
            dynfilters: DynFilters::default(),
        }
    }

    pub(super) fn generate(&self, mode: Mode) -> Result<String, QueryError> {
        let dynfilters = match mode {
            Mode::Off => None,
            Mode::On => Some(&self.dynfilters),
        };
        generate(
            self.backend,
            &self.type_map,
            None,
            &self.queries,
            1,
            dynfilters,
        )
        .map(|tokens| tokens.to_string())
    }
}

fn caches(backend: DbCrate) -> bool {
    !matches!(backend, DbCrate::Postgres(Postgres::Sync | Postgres::Tokio))
}

/// `$n`, `?`, or `?n`: what sqlc reports for the engine.
fn ph(backend: DbCrate, number: usize) -> String {
    match dialect(backend) {
        crate::dynfilter::Dialect::PostgreSql => format!("${number}"),
        crate::dynfilter::Dialect::MySql => "?".to_string(),
        crate::dynfilter::Dialect::Sqlite => format!("?{number}"),
    }
}

/// `$n` or `?`: what the generated code executes with.
fn runtime_ph(backend: DbCrate, number: usize) -> String {
    match dialect(backend) {
        crate::dynfilter::Dialect::MySql => "?".to_string(),
        _ => format!("${number}"),
    }
}

fn many(name: &str, sql: &str, params: &[(&str, bool)]) -> plugin::Query {
    query(
        name,
        ":many",
        sql,
        vec![column("id", false)],
        params
            .iter()
            .enumerate()
            .map(|(index, (name, slice))| (index as i32 + 1, column(name, *slice)))
            .collect(),
    )
}

fn search(backend: DbCrate, name: &str) -> plugin::Query {
    many(
        name,
        &format!(
            "SELECT id FROM users\nWHERE\n  email = {} -- :if @email\n  AND phone <> '' -- :flag @with_phone",
            ph(backend, 1)
        ),
        &[("email", false)],
    )
}

/// A `sqlc.slice` parameter (MySQL/SQLite) or a PostgreSQL array (`= ANY`) named `ids`.
fn by_ids(backend: DbCrate) -> plugin::Query {
    match dialect(backend) {
        crate::dynfilter::Dialect::PostgreSql => {
            let mut ids = column("ids", false);
            ids.is_array = true;
            ids.array_dims = 1;
            query(
                "ByIds",
                ":many",
                "SELECT id FROM users\nWHERE\n  id = ANY($1::bigint[]) -- :if @ids",
                vec![column("id", false)],
                vec![(1, ids)],
            )
        }
        _ => many(
            "ByIds",
            &format!(
                "SELECT id FROM users\nWHERE\n  id IN (/*SLICE:ids*/{}) -- :if @ids\n  AND email = {} -- :if @email",
                ph(backend, 1),
                ph(backend, 2)
            ),
            &[("ids", true), ("email", false)],
        ),
    }
}

pub(super) fn matrix(backend: DbCrate) -> Fixture {
    let mut fixture = Fixture::new(
        backend,
        vec![
            search(backend, "SearchUsers"),
            search(backend, "SkippedExpand"),
            search(backend, "SkippedCache"),
            by_ids(backend),
            query(
                "GetUser",
                ":one",
                &format!("SELECT id FROM users WHERE id = {}", ph(backend, 1)),
                vec![column("id", false)],
                vec![(1, column("id", false))],
            ),
        ],
    );
    fixture.dynfilters.variants_skip = vec!["SkippedExpand".to_string()];
    fixture.dynfilters.prepared_skip = vec!["SkippedCache".to_string()];
    fixture
}

pub(super) fn region<'a>(tokens: &'a str, start: &str, end: &str) -> &'a str {
    let from = tokens
        .find(start)
        .unwrap_or_else(|| panic!("{start} in {tokens}"));
    let to = tokens[from..]
        .find(end)
        .map_or(tokens.len(), |to| from + to);
    &tokens[from..to]
}

#[test]
fn eligible_queries_emit_variants_and_warmups() {
    for backend in DbCrate::ALL {
        let tokens = matrix(backend).generate(Mode::On).unwrap();
        let states = region(&tokens, "pub const SEARCH_USERS_STATES", "] ;");
        // Gate bits count the distinct condition sets in line order: `email`, then
        // `with_phone`; states are sorted by their bits.
        assert_eq!(
            states,
            "pub const SEARCH_USERS_STATES : & [dynfilter :: State] = & [dynfilter :: State { gates : 0 , sql : SEARCH_USERS_VARIANTS [0] , binds : & [] } , dynfilter :: State { gates : 1 , sql : SEARCH_USERS_VARIANTS [2] , binds : & [dynfilter :: Bind :: Arg (0usize) ,] } , dynfilter :: State { gates : 2 , sql : SEARCH_USERS_VARIANTS [1] , binds : & [] } , dynfilter :: State { gates : 3 , sql : SEARCH_USERS_VARIANTS [3] , binds : & [dynfilter :: Bind :: Arg (0usize) ,] } ,",
            "{backend}"
        );
        let constant = region(&tokens, "pub const SEARCH_USERS_VARIANTS", "] ;");
        let expected = [
            "SELECT id FROM users".to_string(),
            "SELECT id FROM users\nWHERE\n  phone <> ''".to_string(),
            format!(
                "SELECT id FROM users\nWHERE\n  email = {}",
                runtime_ph(backend, 1)
            ),
            format!(
                "SELECT id FROM users\nWHERE\n  email = {}\n  AND phone <> ''",
                runtime_ph(backend, 1)
            ),
        ];
        let positions = expected
            .iter()
            .map(|text| {
                constant
                    .find(&format!("\"{text}\""))
                    .unwrap_or_else(|| panic!("{backend}: {text:?} in {constant}"))
            })
            .collect::<Vec<_>>();
        assert!(
            positions.windows(2).all(|pair| pair[0] < pair[1]),
            "{backend}"
        );
        assert_eq!(constant.matches("r\"").count(), 4, "{backend}: {constant}");
        for absent in [
            "SKIPPED_EXPAND_VARIANTS",
            "SKIPPED_CACHE_VARIANTS",
            "GET_USER_VARIANTS",
            "prepare_skipped_expand",
            "prepare_skipped_cache",
        ] {
            assert!(!tokens.contains(absent), "{backend}: {absent}");
        }
        let pg_arrays = matches!(dialect(backend), crate::dynfilter::Dialect::PostgreSql);
        assert_eq!(tokens.contains("BY_IDS_VARIANTS"), pg_arrays, "{backend}");
        assert!(tokens.contains("pub const GET_USER : & str"), "{backend}");

        let count = if pg_arrays { 6 } else { 4 };
        if caches(backend) {
            assert!(
                tokens.contains("fn prepare_search_users ("),
                "{backend}: {tokens}"
            );
            assert_eq!(
                tokens.contains("fn prepare_by_ids ("),
                pg_arrays,
                "{backend}"
            );
            let aggregate = region(&tokens, "fn prepare_dynfilter_variants", "Ok (())");
            assert!(
                aggregate.contains("self :: prepare_search_users ("),
                "{aggregate}"
            );
            assert_eq!(aggregate.contains("self :: prepare_by_ids ("), pg_arrays);
            assert!(!aggregate.contains("skipped"), "{aggregate}");
            assert!(
                tokens.contains(&format!(
                    "pub const DYNFILTER_VARIANT_COUNT : usize = {count}"
                )),
                "{backend}: {tokens}"
            );
        } else {
            // `prepare_get_user` is the static query's existing statement helper, not a warm-up.
            for absent in [
                "prepare_search_users",
                "prepare_dynfilter_variants",
                "DYNFILTER_VARIANT_COUNT",
                "prepare_cached",
            ] {
                assert!(!tokens.contains(absent), "{backend}: {absent}");
            }
            assert!(tokens.contains("(& sql , & values)"), "{backend}");
        }
    }
}

#[test]
fn eligible_queries_use_the_backend_cache_and_ineligible_ones_do_not() {
    for backend in DbCrate::ALL {
        let tokens = matrix(backend).generate(Mode::On).unwrap();
        let pg_arrays = matches!(dialect(backend), crate::dynfilter::Dialect::PostgreSql);
        let eligible = region(&tokens, "fn search_users", "pub const SKIPPED_EXPAND");
        let ineligible = region(&tokens, "pub const SKIPPED_EXPAND", "pub const BY_IDS");
        // PostgreSQL arrays keep the SQL text fixed, so `ByIds` is cached there and not on
        // the slice-expanding engines.
        let by_ids = region(&tokens, "pub const BY_IDS", "pub const GET_USER");
        match backend {
            DbCrate::Sqlx(_) => {
                assert!(!eligible.contains("persistent"), "{backend}: {eligible}");
                assert_eq!(
                    ineligible.matches("persistent (false)").count(),
                    2,
                    "{backend}"
                );
                assert_eq!(
                    by_ids.contains("persistent (false)"),
                    !pg_arrays,
                    "{backend}"
                );
            }
            DbCrate::Rusqlite => {
                assert!(eligible.contains("prepare_cached (sql)"), "{eligible}");
                assert_eq!(ineligible.matches("prepare (& sql)").count(), 2);
                assert!(!ineligible.contains("prepare_cached (sql)"));
                assert!(by_ids.contains("prepare (& sql)"));
            }
            DbCrate::Postgres(Postgres::Deadpool) => {
                for cached in [eligible, by_ids] {
                    assert_eq!(
                        cached
                            .matches("let statement = client . prepare_cached (sql) . await ?")
                            .count(),
                        2,
                        "{cached}"
                    );
                    assert_eq!(cached.matches("(& statement , & values)").count(), 1);
                    assert_eq!(
                        cached.matches("query_raw (& statement , values)").count(),
                        1
                    );
                }
                assert!(!ineligible.contains("prepare_cached"), "{ineligible}");
                assert_eq!(ineligible.matches("(& sql , & values)").count(), 2);
                assert_eq!(ineligible.matches("query_raw (& sql , values)").count(), 2);
            }
            DbCrate::Postgres(_) => {
                assert!(!eligible.contains("prepare_cached"), "{eligible}");
                assert!(eligible.contains("(sql , & values)"));
            }
        }
        // A cached query looks its text up by control state instead of rendering it.
        let looks_up = |region: &str| {
            region.contains(". state (")
                && region.contains("_STATES , & args)")
                && !region.contains(". build (& args)")
        };
        assert!(looks_up(eligible), "{backend}: {eligible}");
        assert_eq!(looks_up(by_ids), pg_arrays, "{backend}: {by_ids}");
        assert!(
            ineligible.contains(". build (& args)"),
            "{backend}: {ineligible}"
        );
        assert!(!ineligible.contains("_STATES"), "{backend}: {ineligible}");
        let static_query = region(&tokens, "pub const GET_USER :", "pub const QUERIES");
        assert!(!static_query.contains("persistent") && !static_query.contains("& sql)"));
    }
}

#[test]
fn off_mode_and_packages_without_dynamic_queries_emit_no_per_query_artifacts() {
    for backend in DbCrate::ALL {
        let fixture = matrix(backend);
        let static_only = Fixture::new(backend, fixture.queries[4..].to_vec());
        let tokens = static_only.generate(Mode::On).unwrap();
        assert!(!tokens.contains("_VARIANTS"), "{backend}");
        assert!(!tokens.contains("prepare_search_users"), "{backend}");
        if caches(backend) {
            let aggregate = region(&tokens, "fn prepare_dynfilter_variants", "Ok (())");
            assert!(aggregate.contains("(_c"), "{aggregate}");
            assert!(!aggregate.contains("self ::"), "{aggregate}");
            assert!(tokens.contains("pub const DYNFILTER_VARIANT_COUNT : usize = 0"));
        } else {
            assert!(!tokens.contains("prepare_dynfilter_variants"), "{backend}");
        }

        let off = fixture.generate(Mode::Off).unwrap();
        for absent in [
            "_VARIANTS",
            "prepare_dynfilter_variants",
            "DYNFILTER_VARIANT_COUNT",
        ] {
            assert!(!off.contains(absent), "{backend}: {absent}");
        }
        match backend {
            DbCrate::Sqlx(_) => assert_eq!(off.matches("persistent (false)").count(), 4),
            DbCrate::Rusqlite => assert_eq!(off.matches("prepare (& sql)").count(), 4),
            DbCrate::Postgres(_) => assert!(!off.contains("prepare_cached (sql")),
        }
    }
}

#[test]
fn reserves_prepared_identifiers_only_where_emitted() {
    for backend in DbCrate::ALL {
        let colliding = |name: &str, ident: &str, mode: Mode, collides: bool| {
            let mut fixture = matrix(backend);
            fixture.queries.push(query(
                name,
                ":many",
                &format!("SELECT id FROM users WHERE id = {}", ph(backend, 1)),
                vec![column("id", false)],
                vec![(1, column("id", false))],
            ));
            let result = fixture.generate(mode);
            match (collides, result) {
                (true, Err(error)) => {
                    let error = error.to_string();
                    assert!(
                        error.contains(&format!("`{ident}`")),
                        "{backend} {name}: {error}"
                    );
                }
                (true, Ok(tokens)) => panic!("{backend} {name} generated: {tokens}"),
                (false, Ok(_)) => {}
                (false, Err(error)) => panic!("{backend} {name} {mode:?}: {error}"),
            }
        };
        colliding(
            "SearchUsersVariants",
            "SEARCH_USERS_VARIANTS",
            Mode::On,
            true,
        );
        colliding(
            "SearchUsersVariants",
            "SEARCH_USERS_VARIANTS",
            Mode::Off,
            false,
        );
        colliding(
            "PrepareSearchUsers",
            "prepare_search_users",
            Mode::On,
            caches(backend),
        );
        colliding(
            "PrepareSearchUsers",
            "prepare_search_users",
            Mode::Off,
            false,
        );
        colliding(
            "PrepareDynfilterVariants",
            "prepare_dynfilter_variants",
            Mode::On,
            caches(backend),
        );
        colliding(
            "DynfilterVariantCount",
            "DYNFILTER_VARIANT_COUNT",
            Mode::On,
            caches(backend),
        );
        colliding(
            "PrepareDynfilterVariants",
            "prepare_dynfilter_variants",
            Mode::Off,
            false,
        );

        let mut fixture = matrix(backend);
        fixture.queries.push(search(backend, "DynfilterVariants"));
        let result = fixture.generate(Mode::On);
        if caches(backend) {
            let error = result.unwrap_err().to_string();
            assert!(
                error.contains("`prepare_dynfilter_variants`"),
                "{backend}: {error}"
            );
        } else {
            result.unwrap();
        }
    }
}

#[test]
fn slice_and_skipped_queries_still_expand() {
    for backend in DbCrate::ALL {
        let fixture = matrix(backend);
        let (_, queries) = parsed(backend, &fixture.type_map, None, &fixture.queries);
        let expected = expand_all(
            &queries,
            &Options {
                placeholders: backend.runtime_placeholders(),
                dialect: backend.runtime_dialect(),
                limit: 1024,
                skip: &fixture.dynfilters.variants_skip,
                cache_skip: &[],
                bind_classes: None,
            },
        )
        .unwrap();
        let names = expected
            .iter()
            .map(|expansion| expansion.query.query_name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, ["SearchUsers", "SkippedCache", "ByIds"], "{backend}");
    }
}

#[test]
fn deadpool_caches_every_supported_annotation_and_flag_only_queries() {
    let dynamic = |name: &str, cmd: &str, sql: &str| {
        query(
            name,
            cmd,
            sql,
            vec![column("id", false)],
            vec![(1, column("email", false))],
        )
    };
    let sql = "SELECT id FROM users\nWHERE\n  email = $1 -- :if @email";
    let write = "UPDATE users SET phone = ''\nWHERE\n  email = $1 -- :if @email";
    let fixture = Fixture::new(
        DbCrate::Postgres(Postgres::Deadpool),
        vec![
            dynamic("One", ":one", sql),
            dynamic("Many", ":many", sql),
            dynamic("Exec", ":exec", write),
            dynamic("Rows", ":execrows", write),
            dynamic("Res", ":execresult", write),
            query(
                "Flagged",
                ":many",
                "SELECT id FROM users\nWHERE\n  phone <> '' -- :flag @with_phone",
                vec![column("id", false)],
                Vec::new(),
            ),
        ],
    );
    let tokens = fixture.generate(Mode::On).unwrap();
    for (start, end, functions) in [
        ("pub const ONE :", "pub const MANY :", 2),
        ("pub const MANY :", "pub const EXEC :", 2),
        ("pub const EXEC :", "pub const ROWS :", 1),
        ("pub const ROWS :", "pub const RES :", 1),
        ("pub const RES :", "pub const FLAGGED :", 1),
        ("pub const FLAGGED :", "pub const QUERIES", 2),
    ] {
        let region = region(&tokens, start, end);
        assert_eq!(
            region
                .matches("let statement = client . prepare_cached (sql) . await ?")
                .count(),
            functions,
            "{start}: {region}"
        );
        assert!(!region.contains("(& sql , & values)"), "{start}: {region}");
        assert!(!region.contains("query_raw (& sql"), "{start}: {region}");
        assert!(region.contains("_VARIANTS : & [& str]"), "{start}");
        assert!(
            region.contains("_STATES : & [dynfilter :: State]"),
            "{start}"
        );
        assert!(region.contains("fn prepare_"), "{start}");
    }
    assert!(
        tokens.contains("pub const DYNFILTER_VARIANT_COUNT : usize = 12"),
        "{tokens}"
    );
    let flagged = region(&tokens, "pub const FLAGGED_VARIANTS", "] ;");
    assert!(
        flagged.contains(
            "r\"SELECT id FROM users\" , r\"SELECT id FROM users\nWHERE\n  phone <> ''\""
        ),
        "{flagged}"
    );

    // The same flag-only query on sqlx takes the no-bind branch of the setup, still cached.
    let fixture = Fixture::new(DbCrate::Sqlx(Sqlx::Postgres), fixture.queries[5..].to_vec());
    let tokens = fixture.generate(Mode::On).unwrap();
    let function = region(
        &tokens,
        "pub async fn flagged",
        "pub async fn prepare_flagged",
    );
    assert!(
        function.contains("debug_assert ! (binds . is_empty ()"),
        "{function}"
    );
    assert!(!function.contains("persistent"), "{function}");
    let warmup = region(&tokens, "pub async fn prepare_flagged (", "Ok (())");
    assert_eq!(warmup.matches("& []) . await ?").count(), 2, "{warmup}");
    assert!(tokens.contains("pub const DYNFILTER_VARIANT_COUNT : usize = 2"));
}

/// Past 128 distinct gates there is no `X_STATES` table: the query stays eligible and cached
/// but renders its text per call, on every backend.
#[test]
fn queries_with_more_than_128_gates_keep_the_render_path() {
    let choices = (0..129).map(|n| format!("c{n}")).collect::<Vec<_>>();
    let cases = choices
        .iter()
        .enumerate()
        .map(|(n, choice)| {
            let comma = if n + 1 < choices.len() { "," } else { "" };
            format!("  id + {n} ASC{comma} -- :case @{choice}")
        })
        .collect::<Vec<_>>()
        .join("\n");
    let sql = format!(
        "SELECT id FROM users\nORDER BY -- :switch @sort {} default=c0\n{cases}",
        choices.join(" ")
    );
    for backend in DbCrate::ALL {
        let fixture = Fixture::new(
            backend,
            vec![query(
                "Many",
                ":many",
                &sql,
                vec![column("id", false)],
                vec![],
            )],
        );
        let (_, queries) = parsed(backend, &fixture.type_map, None, &fixture.queries);
        let expanded = expand_all(
            &queries,
            &Options {
                placeholders: backend.runtime_placeholders(),
                dialect: backend.runtime_dialect(),
                limit: 1024,
                skip: &[],
                cache_skip: &[],
                bind_classes: None,
            },
        )
        .unwrap();
        assert_eq!(expanded[0].variants.len(), 129, "{backend}");
        assert!(expanded[0].states.is_none(), "{backend}");

        let tokens = fixture.generate(Mode::On).unwrap();
        assert!(tokens.contains("pub const MANY_VARIANTS"), "{backend}");
        assert!(!tokens.contains("_STATES"), "{backend}");
        assert!(tokens.contains(". build (& args)"), "{backend}");
        assert!(!tokens.contains(". state ("), "{backend}");
        match backend {
            DbCrate::Rusqlite => assert!(tokens.contains("prepare_cached (& sql)"), "{tokens}"),
            DbCrate::Postgres(Postgres::Deadpool) => {
                assert!(tokens.contains("-> (String , Vec <"), "{tokens}");
                assert!(
                    tokens.contains("prepare_cached (& sql) . await ?"),
                    "{tokens}"
                );
            }
            DbCrate::Postgres(_) => assert!(tokens.contains("(& sql , & values)"), "{tokens}"),
            DbCrate::Sqlx(_) => {
                assert!(tokens.contains("(& sql)"), "{tokens}");
                assert!(!tokens.contains("persistent (false)"), "{tokens}");
            }
        }
    }
}

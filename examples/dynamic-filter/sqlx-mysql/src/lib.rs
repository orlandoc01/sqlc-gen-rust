#[allow(dead_code)]
mod queries;

#[cfg(test)]
mod tests {
    use super::queries;
    use test_context::test_context;
    use test_utils::SqlxMysqlContext;

    async fn migrate(pool: &sqlx::MySqlPool) {
        sqlx::raw_sql(include_str!("../schema.sql"))
            .execute(pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO users (id, email, phone) VALUES (1, 'alice@example.com', '111'), (2, 'bob@example.com', '222'), (3, 'carol@example.com', '333')",
        )
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO orders (id, user_id, created_at) VALUES (1, 1, '2024-01-01'), (2, 2, '2025-01-01')",
        )
        .execute(pool)
        .await
        .unwrap();
    }

    /// The application lifecycle: migrate through a plain pool, then connect the pool every
    /// test uses with enough cache capacity and a warm-up on each new connection.
    async fn warmed(ctx: &mut SqlxMysqlContext) -> &sqlx::MySqlPool {
        migrate(&ctx.pool).await;
        ctx.rebuild_pool(
            |options| options.statement_cache_capacity(queries::DYNFILTER_VARIANT_COUNT + 16),
            |pool| {
                pool.after_connect(|conn, _| Box::pin(queries::prepare_dynfilter_variants(conn)))
            },
        )
        .await;
        &ctx.pool
    }

    fn params() -> queries::SearchUsersParams<'static> {
        queries::SearchUsersParams {
            limit: 100,
            ..Default::default()
        }
    }

    fn count_users_params() -> queries::CountUsersParams<'static> {
        Default::default()
    }

    #[test_context(SqlxMysqlContext)]
    #[tokio::test]
    async fn searches_without_filters(ctx: &mut SqlxMysqlContext) {
        let pool = warmed(ctx).await;

        let users = queries::search_users(pool, params()).await.unwrap();
        assert_eq!(users.len(), 3);
    }

    #[test_context(SqlxMysqlContext)]
    #[tokio::test]
    async fn applies_each_scalar_filter(ctx: &mut SqlxMysqlContext) {
        let pool = warmed(ctx).await;

        let users = queries::search_users(
            pool,
            queries::SearchUsersParams {
                email: Some("alice@example.com"),
                ..params()
            },
        )
        .await
        .unwrap();
        assert_eq!(users.iter().map(|user| user.id).collect::<Vec<_>>(), [1]);

        let users = queries::search_users(
            pool,
            queries::SearchUsersParams {
                phone: Some("222"),
                ..params()
            },
        )
        .await
        .unwrap();
        assert_eq!(users.iter().map(|user| user.id).collect::<Vec<_>>(), [2]);
    }

    #[test_context(SqlxMysqlContext)]
    #[tokio::test]
    async fn distinguishes_none_empty_and_populated_slices(ctx: &mut SqlxMysqlContext) {
        let pool = warmed(ctx).await;

        assert_eq!(
            queries::search_users(pool, params()).await.unwrap().len(),
            3
        );

        let empty = [];
        let users = queries::search_users(
            pool,
            queries::SearchUsersParams {
                ids: Some(&empty),
                ..params()
            },
        )
        .await
        .unwrap();
        assert!(users.is_empty());

        let ids = [1, 3];
        let users = queries::search_users(
            pool,
            queries::SearchUsersParams {
                ids: Some(&ids),
                ..params()
            },
        )
        .await
        .unwrap();
        assert_eq!(users.iter().map(|user| user.id).collect::<Vec<_>>(), [1, 3]);
        assert_eq!(queries::dynfilter::nilable(&empty), None);
    }

    #[test_context(SqlxMysqlContext)]
    #[tokio::test]
    async fn toggles_the_orders_block_and_its_filter(ctx: &mut SqlxMysqlContext) {
        let pool = warmed(ctx).await;

        let users = queries::search_users(
            pool,
            queries::SearchUsersParams {
                has_orders: true,
                ..params()
            },
        )
        .await
        .unwrap();
        assert_eq!(users.iter().map(|user| user.id).collect::<Vec<_>>(), [1, 2]);

        let users = queries::search_users(
            pool,
            queries::SearchUsersParams {
                has_orders: true,
                created_at: Some("2025-01-01"),
                ..params()
            },
        )
        .await
        .unwrap();
        assert_eq!(users.iter().map(|user| user.id).collect::<Vec<_>>(), [2]);
    }

    #[test_context(SqlxMysqlContext)]
    #[tokio::test]
    async fn selects_each_sort_preset(ctx: &mut SqlxMysqlContext) {
        let pool = warmed(ctx).await;

        let asc = queries::search_users(
            pool,
            queries::SearchUsersParams {
                sort: queries::SearchUsersSort::IdAsc,
                ..params()
            },
        )
        .await
        .unwrap();
        assert_eq!(
            asc.iter().map(|user| user.id).collect::<Vec<_>>(),
            [1, 2, 3]
        );

        let desc = queries::search_users(
            pool,
            queries::SearchUsersParams {
                sort: queries::SearchUsersSort::IdDesc,
                ..params()
            },
        )
        .await
        .unwrap();
        assert_eq!(
            desc.iter().map(|user| user.id).collect::<Vec<_>>(),
            [3, 2, 1]
        );

        let shortest = queries::search_users(
            pool,
            queries::SearchUsersParams {
                sort: queries::SearchUsersSort::ShortestEmail,
                ..params()
            },
        )
        .await
        .unwrap();
        assert_eq!(
            shortest.iter().map(|user| user.id).collect::<Vec<_>>(),
            [2, 3, 1]
        );

        assert_eq!(
            queries::SearchUsersSort::default(),
            queries::SearchUsersSort::IdAsc
        );

        let by_default = queries::search_users(pool, params()).await.unwrap();
        assert_eq!(
            by_default.iter().map(|user| user.id).collect::<Vec<_>>(),
            [1, 2, 3]
        );
    }

    #[test_context(SqlxMysqlContext)]
    #[tokio::test]
    async fn counts_users_with_dynamic_filters(ctx: &mut SqlxMysqlContext) {
        let pool = warmed(ctx).await;

        assert_eq!(
            queries::count_users(pool, count_users_params())
                .await
                .unwrap()
                .total,
            3
        );
        assert_eq!(
            queries::count_users(
                pool,
                queries::CountUsersParams {
                    email: Some("alice@example.com"),
                    ..count_users_params()
                },
            )
            .await
            .unwrap()
            .total,
            1
        );

        let empty = [];
        assert_eq!(
            queries::count_users(
                pool,
                queries::CountUsersParams {
                    ids: Some(&empty),
                    ..count_users_params()
                },
            )
            .await
            .unwrap()
            .total,
            0
        );
        assert_eq!(
            queries::count_users(
                pool,
                queries::CountUsersParams {
                    ids: None,
                    ..count_users_params()
                },
            )
            .await
            .unwrap()
            .total,
            3
        );
        assert!(
            queries::count_users_opt(pool, count_users_params())
                .await
                .unwrap()
                .is_some()
        );
    }

    #[test_context(SqlxMysqlContext)]
    #[tokio::test]
    async fn touches_users_with_dynamic_filters(ctx: &mut SqlxMysqlContext) {
        let pool = warmed(ctx).await;

        assert_eq!(
            queries::touch_users(pool, Default::default())
                .await
                .unwrap(),
            3
        );
        assert_eq!(
            queries::touch_users(
                pool,
                queries::TouchUsersParams {
                    email: Some("alice@example.com"),
                    ..Default::default()
                },
            )
            .await
            .unwrap(),
            1
        );

        let empty = [];
        assert_eq!(
            queries::touch_users(
                pool,
                queries::TouchUsersParams {
                    ids: Some(&empty),
                    ..Default::default()
                },
            )
            .await
            .unwrap(),
            0
        );
        assert_eq!(
            queries::touch_users(
                pool,
                queries::TouchUsersParams {
                    ids: None,
                    ..Default::default()
                },
            )
            .await
            .unwrap(),
            3
        );
    }

    #[test_context(SqlxMysqlContext)]
    #[tokio::test]
    async fn combines_where_and_order_switches(ctx: &mut SqlxMysqlContext) {
        use queries::{ListUsersByScopeOrder as Order, ListUsersByScopeScope as Scope};
        let pool = warmed(ctx).await;
        let list = |scope, order, min_id, with_phone| async move {
            queries::list_users_by_scope(
                pool,
                queries::ListUsersByScopeParams {
                    min_id,
                    scope,
                    with_phone,
                    order,
                },
            )
            .await
            .unwrap()
            .iter()
            .map(|user| user.id)
            .collect::<Vec<_>>()
        };

        assert_eq!(
            list(Scope::Everyone, Order::Oldest, None, false).await,
            [1, 2, 3]
        );
        assert_eq!(
            list(Scope::WithOrders, Order::Newest, None, true).await,
            [2, 1]
        );
        assert_eq!(
            list(Scope::WithoutOrders, Order::Oldest, None, false).await,
            [3]
        );
        assert_eq!(
            list(Scope::Everyone, Order::Newest, Some(2), false).await,
            [3, 2]
        );
        let by_default = queries::list_users_by_scope(pool, Default::default())
            .await
            .unwrap();
        assert_eq!(
            by_default.iter().map(|user| user.id).collect::<Vec<_>>(),
            [1, 2, 3]
        );
    }

    #[test_context(SqlxMysqlContext)]
    #[tokio::test]
    async fn gates_a_join_with_its_bind_and_ordering(ctx: &mut SqlxMysqlContext) {
        let pool = warmed(ctx).await;
        sqlx::raw_sql("INSERT INTO users (id, email, phone) VALUES (4, 'dave@example.com', ''), (5, 'erin@example.com', '555'); INSERT INTO orders (id, user_id, created_at) VALUES (3, 4, '2025-06-01'), (4, 1, '2025-02-01');").execute(pool).await.unwrap();
        // Dave has no phone but an order, Erin has a phone and no order, and Alice has two
        // orders, so the phone flag, the inner join's multiplicity, and an enabled join with
        // no match each change the result.
        for (orders_since, with_phone, expected) in [
            (None, false, vec![1, 2, 3, 4, 5]),
            (None, true, vec![1, 2, 3, 5]),
            (Some("2024-01-01"), false, vec![4, 1, 2, 1]),
            (Some("2025-01-01"), true, vec![1, 2]),
            (Some("2030-01-01"), false, vec![]),
        ] {
            let users = queries::search_users_with_orders(
                pool,
                queries::SearchUsersWithOrdersParams {
                    orders_since,
                    with_phone,
                },
            )
            .await
            .unwrap();
            assert_eq!(
                users.iter().map(|user| user.id).collect::<Vec<_>>(),
                expected
            );
        }
    }

    #[test_context(SqlxMysqlContext)]
    #[tokio::test]
    async fn resolves_a_whole_term_output_alias_in_order_by(ctx: &mut SqlxMysqlContext) {
        let pool = warmed(ctx).await;
        for (with_orders, expected) in [(false, vec![1, 2, 3]), (true, vec![2, 1])] {
            let users = queries::list_users_ordered_by_alias(
                pool,
                queries::ListUsersOrderedByAliasParams { with_orders },
            )
            .await
            .unwrap();
            assert_eq!(
                users.iter().map(|user| user.id).collect::<Vec<_>>(),
                expected
            );
        }
    }

    #[test_context(SqlxMysqlContext)]
    #[tokio::test]
    async fn gates_or_operands_with_a_false_fallback(ctx: &mut SqlxMysqlContext) {
        let pool = warmed(ctx).await;
        for (email_pattern, phone_pattern, expected) in [
            (None, None, vec![]),
            (Some("alice%"), None, vec![1]),
            (None, Some("%3"), vec![3]),
            (Some("alice%"), Some("222"), vec![1, 2]),
        ] {
            let users = queries::search_users_by_pattern(
                pool,
                queries::SearchUsersByPatternParams {
                    email_pattern,
                    phone_pattern,
                },
            )
            .await
            .unwrap();
            assert_eq!(
                users.iter().map(|user| user.id).collect::<Vec<_>>(),
                expected
            );
        }
    }

    fn pattern_states() -> [queries::SearchUsersByPatternParams<'static>; 4] {
        [
            queries::SearchUsersByPatternParams {
                email_pattern: None,
                phone_pattern: None,
            },
            queries::SearchUsersByPatternParams {
                email_pattern: None,
                phone_pattern: Some("2%"),
            },
            queries::SearchUsersByPatternParams {
                email_pattern: Some("a%"),
                phone_pattern: None,
            },
            queries::SearchUsersByPatternParams {
                email_pattern: Some("%@example.com"),
                phone_pattern: Some("3%"),
            },
        ]
    }

    fn distinct_variants() -> usize {
        [
            queries::LIST_USERS_BY_SCOPE_VARIANTS,
            queries::SEARCH_USERS_WITH_ORDERS_VARIANTS,
            queries::SEARCH_USERS_BY_PATTERN_VARIANTS,
            queries::LIST_USERS_ORDERED_BY_ALIAS_VARIANTS,
            queries::SEARCH_USERS_WITH_HASH_COMMENT_VARIANTS,
        ]
        .concat()
        .into_iter()
        .collect::<std::collections::HashSet<_>>()
        .len()
    }

    #[test_context(SqlxMysqlContext)]
    #[tokio::test]
    async fn warms_every_pooled_connection_and_reuses_prepared_statements(
        ctx: &mut SqlxMysqlContext,
    ) {
        use sqlx::Connection as _;
        let pool = warmed(ctx).await;
        let mut first = pool.acquire().await.unwrap();
        let mut second = pool.acquire().await.unwrap();
        let warmed = distinct_variants();
        assert_eq!(warmed, queries::DYNFILTER_VARIANT_COUNT);
        for conn in [&mut *first, &mut *second] {
            assert_eq!(conn.cached_statements_size(), warmed);
        }
        assert_eq!(queries::SEARCH_USERS_BY_PATTERN_VARIANTS.len(), 4);

        for _ in 0..2 {
            for params in pattern_states() {
                queries::search_users_by_pattern(&mut *first, params)
                    .await
                    .unwrap();
            }
            for params in pattern_states() {
                queries::search_users_by_pattern(&mut *second, params)
                    .await
                    .unwrap();
            }
        }
        assert_eq!(first.cached_statements_size(), warmed);

        // Expanding-slice queries are not cache-eligible and bypass the cache entirely.
        let ids = [1, 2];
        queries::search_users(
            &mut *first,
            queries::SearchUsersParams {
                ids: Some(&ids),
                ..params()
            },
        )
        .await
        .unwrap();
        assert_eq!(first.cached_statements_size(), warmed);

        queries::prepare_dynfilter_variants(&mut first)
            .await
            .unwrap();
        assert_eq!(first.cached_statements_size(), warmed);

        // A key that differs by one byte would show up as growth, so the checks above can
        // only pass when the runtime renders the warmed text exactly.
        let mismatched = format!("{} ", queries::SEARCH_USERS_BY_PATTERN_VARIANTS[0]);
        sqlx::Executor::prepare(&mut *first, mismatched.as_str())
            .await
            .unwrap();
        assert_eq!(first.cached_statements_size(), warmed + 1);
    }

    /// The session's `Com_stmt_prepare` counter, read over the text protocol so the probe
    /// itself prepares nothing.
    async fn prepares(conn: &mut sqlx::MySqlConnection) -> u64 {
        use sqlx::Row as _;
        let rows = sqlx::raw_sql("SHOW SESSION STATUS LIKE 'Com_stmt_prepare'")
            .fetch_all(conn)
            .await
            .unwrap();
        rows[0].get::<String, _>("Value").parse().unwrap()
    }

    /// Reuse proven by the server counter: warmed states execute without a new prepare, while
    /// the expanding-slice query (uncached by design) prepares on every call.
    #[test_context(SqlxMysqlContext)]
    #[tokio::test]
    async fn executes_warmed_states_without_preparing_again(ctx: &mut SqlxMysqlContext) {
        let pool = warmed(ctx).await;
        let mut conn = pool.acquire().await.unwrap();
        let before = prepares(&mut conn).await;
        for _ in 0..2 {
            for params in pattern_states() {
                queries::search_users_by_pattern(&mut *conn, params)
                    .await
                    .unwrap();
            }
        }
        assert_eq!(prepares(&mut conn).await, before);
        let ids = [1, 2];
        queries::search_users(
            &mut *conn,
            queries::SearchUsersParams {
                ids: Some(&ids),
                ..params()
            },
        )
        .await
        .unwrap();
        assert_eq!(prepares(&mut conn).await, before + 1);
    }

    #[test_context(SqlxMysqlContext)]
    #[tokio::test]
    async fn an_undersized_or_disabled_cache_still_executes(ctx: &mut SqlxMysqlContext) {
        use sqlx::Connection as _;
        warmed(ctx).await;
        for capacity in [0, 2] {
            ctx.rebuild_pool(
                |options| options.statement_cache_capacity(capacity),
                |pool| {
                    pool.after_connect(|conn, _| {
                        Box::pin(queries::prepare_dynfilter_variants(conn))
                    })
                },
            )
            .await;
            let pool = &ctx.pool;
            let mut conn = pool.acquire().await.unwrap();
            assert!(conn.cached_statements_size() <= capacity);
            for params in pattern_states() {
                queries::search_users_by_pattern(&mut *conn, params)
                    .await
                    .unwrap();
            }
            assert_eq!(
                queries::search_users(&mut *conn, params())
                    .await
                    .unwrap()
                    .len(),
                3
            );
        }
    }

    /// Release-mode timing. `uncached` runs the generated cache-skipped twin of the fixture
    /// (same SQL, `persistent(false)`), `first use` relies on caching without a warm-up, and
    /// `warmed` runs the aggregate warm-up on connection creation. Migration and seeding happen
    /// before any timer starts; the workload cycles the fixture's four shapes on one connection.
    /// Repeat with `cargo test --release -p dynamic-filter-sqlx-mysql -- --ignored --nocapture`; the recorded
    /// medians live in `examples/dynamic-filter/PREPARED_BENCHMARKS.md`.
    #[test_context(SqlxMysqlContext)]
    #[tokio::test]
    #[ignore]
    async fn measures_prepared_statement_reuse(ctx: &mut SqlxMysqlContext) {
        use sqlx::Connection as _;
        const ROUNDS: usize = 200;
        const CAPACITY: usize = 128;
        let hot = || queries::SearchUsersByPatternParams {
            email_pattern: Some("a%"),
            phone_pattern: None,
        };
        let twin = |params: queries::SearchUsersByPatternParams<'static>| {
            queries::SearchUsersByPatternUncachedParams {
                email_pattern: params.email_pattern,
                phone_pattern: params.phone_pattern,
            }
        };
        migrate(&ctx.pool).await;
        let mut report = Vec::new();
        for (label, uncached, warm) in [
            ("uncached (skipped twin)", true, false),
            ("cache on first use", false, false),
            ("warmed", false, true),
        ] {
            let started = std::time::Instant::now();
            ctx.rebuild_pool(
                |options| options.statement_cache_capacity(CAPACITY),
                |pool| {
                    let pool = pool.min_connections(1).max_connections(1);
                    if warm {
                        pool.after_connect(|conn, _| {
                            Box::pin(queries::prepare_dynfilter_variants(conn))
                        })
                    } else {
                        pool
                    }
                },
            )
            .await;
            let pool = &ctx.pool;
            let mut conn = pool.acquire().await.unwrap();
            let connect = started.elapsed();
            async fn run(
                conn: &mut sqlx::MySqlConnection,
                uncached: bool,
                params: queries::SearchUsersByPatternParams<'static>,
                twin: impl Fn(
                    queries::SearchUsersByPatternParams<'static>,
                ) -> queries::SearchUsersByPatternUncachedParams<'static>,
            ) -> usize {
                if uncached {
                    queries::search_users_by_pattern_uncached(&mut *conn, twin(params))
                        .await
                        .unwrap()
                        .len()
                } else {
                    queries::search_users_by_pattern(&mut *conn, params)
                        .await
                        .unwrap()
                        .len()
                }
            }
            let started = std::time::Instant::now();
            run(&mut conn, uncached, hot(), twin).await;
            let first = started.elapsed();
            let started = std::time::Instant::now();
            for _ in 0..ROUNDS {
                run(&mut conn, uncached, hot(), twin).await;
            }
            let hot_query = started.elapsed() / ROUNDS as u32;
            let started = std::time::Instant::now();
            for _ in 0..ROUNDS {
                for params in pattern_states() {
                    run(&mut conn, uncached, params, twin).await;
                }
            }
            let mixed = started.elapsed() / (ROUNDS * 4) as u32;
            report.push(format!(
                "{label:<24} connect+warm {connect:>9.2?} first {first:>9.2?} hot {hot_query:>9.2?}/call mixed {mixed:>9.2?}/call cached {} capacity {CAPACITY}",
                conn.cached_statements_size()
            ));
        }
        println!(
            "dynamic-filter-sqlx-mysql: 1 connection, warmed shapes {} (when warmed), workload shapes 4, rounds {ROUNDS}\n{}",
            queries::DYNFILTER_VARIANT_COUNT,
            report.join("\n")
        );
    }

    /// A `?` inside a `#` comment claims no argument, so the guarded email bind stays optional
    /// on a cold connection and on a warmed one.
    #[test_context(SqlxMysqlContext)]
    #[tokio::test]
    async fn hash_comments_hold_no_binds(ctx: &mut SqlxMysqlContext) {
        migrate(&ctx.pool).await;
        let params = |email| queries::SearchUsersWithHashCommentParams { id: 2, email };
        let ids = |users: Vec<queries::SearchUsersWithHashCommentRow>| {
            users.into_iter().map(|user| user.id).collect::<Vec<_>>()
        };
        for warm in [false, true] {
            if warm {
                ctx.rebuild_pool(
                    |options| {
                        options.statement_cache_capacity(queries::DYNFILTER_VARIANT_COUNT + 16)
                    },
                    |pool| {
                        pool.after_connect(|conn, _| {
                            Box::pin(queries::prepare_dynfilter_variants(conn))
                        })
                    },
                )
                .await;
            }
            let users = queries::search_users_with_hash_comment(&ctx.pool, params(None))
                .await
                .unwrap();
            assert_eq!(ids(users), [2, 3], "warm={warm}");
            let users = queries::search_users_with_hash_comment(
                &ctx.pool,
                params(Some("carol@example.com")),
            )
            .await
            .unwrap();
            assert_eq!(ids(users), [3], "warm={warm}");
        }
    }
}

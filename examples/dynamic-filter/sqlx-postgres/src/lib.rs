#[allow(dead_code)]
mod queries;

#[cfg(test)]
mod tests {
    use super::queries;
    use test_context::test_context;
    use test_utils::SqlxPgContext;

    async fn migrate(pool: &sqlx::PgPool) {
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
    async fn warmed(ctx: &mut SqlxPgContext) -> &sqlx::PgPool {
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
            row_limit: 100,
            ..Default::default()
        }
    }

    fn count_users_params() -> queries::CountUsersParams<'static> {
        Default::default()
    }

    #[test_context(SqlxPgContext)]
    #[tokio::test]
    async fn searches_without_filters(ctx: &mut SqlxPgContext) {
        let pool = warmed(ctx).await;

        let users = queries::search_users(pool, params()).await.unwrap();
        assert_eq!(users.len(), 3);
    }

    #[test_context(SqlxPgContext)]
    #[tokio::test]
    async fn applies_each_scalar_filter(ctx: &mut SqlxPgContext) {
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

    #[test_context(SqlxPgContext)]
    #[tokio::test]
    async fn distinguishes_none_empty_and_populated_slices(ctx: &mut SqlxPgContext) {
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

    #[test_context(SqlxPgContext)]
    #[tokio::test]
    async fn toggles_the_orders_block_and_its_filter(ctx: &mut SqlxPgContext) {
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
                orders_since: Some("2025-01-01"),
                ..params()
            },
        )
        .await
        .unwrap();
        assert_eq!(users.iter().map(|user| user.id).collect::<Vec<_>>(), [2]);
    }

    #[test_context(SqlxPgContext)]
    #[tokio::test]
    async fn selects_each_sort_preset(ctx: &mut SqlxPgContext) {
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

    #[test_context(SqlxPgContext)]
    #[tokio::test]
    async fn keeps_json_question_operators_beside_dynamic_binds(ctx: &mut SqlxPgContext) {
        let pool = warmed(ctx).await;
        for (key, email, expected) in [
            ("a", None, 3),
            ("z", None, 0),
            ("a", Some("alice@example.com"), 1),
            ("z", Some("alice@example.com"), 0),
        ] {
            let total = queries::count_users_with_json_key(
                pool,
                queries::CountUsersWithJsonKeyParams { key, email },
            )
            .await
            .unwrap()
            .total;
            assert_eq!(total, expected, "{key} {email:?}");
        }
    }

    #[test_context(SqlxPgContext)]
    #[tokio::test]
    async fn counts_users_with_dynamic_filters(ctx: &mut SqlxPgContext) {
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

    #[test_context(SqlxPgContext)]
    #[tokio::test]
    async fn touches_users_with_dynamic_filters(ctx: &mut SqlxPgContext) {
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

    #[test_context(SqlxPgContext)]
    #[tokio::test]
    async fn combines_where_and_order_switches(ctx: &mut SqlxPgContext) {
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

    #[test_context(SqlxPgContext)]
    #[tokio::test]
    async fn gates_a_join_with_its_bind_and_ordering(ctx: &mut SqlxPgContext) {
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

    #[test_context(SqlxPgContext)]
    #[tokio::test]
    async fn resolves_a_whole_term_output_alias_in_order_by(ctx: &mut SqlxPgContext) {
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

    #[test_context(SqlxPgContext)]
    #[tokio::test]
    async fn gates_or_operands_with_a_false_fallback(ctx: &mut SqlxPgContext) {
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

    use sqlx::Connection as _;

    async fn server_statements(conn: &mut sqlx::PgConnection) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM pg_prepared_statements")
            .persistent(false)
            .fetch_one(conn)
            .await
            .unwrap()
    }

    /// `DYNFILTER_VARIANT_COUNT` sums per-query variants; the cache holds one entry per
    /// distinct text, and two zero-bind `COUNT(*)` shapes share theirs.
    fn distinct_variants() -> usize {
        [
            queries::SEARCH_USERS_VARIANTS,
            queries::COUNT_USERS_VARIANTS,
            queries::TOUCH_USERS_VARIANTS,
            queries::LIST_USERS_BY_SCOPE_VARIANTS,
            queries::SEARCH_USERS_WITH_ORDERS_VARIANTS,
            queries::SEARCH_USERS_BY_PATTERN_VARIANTS,
            queries::COUNT_USERS_WITH_JSON_KEY_VARIANTS,
            queries::LIST_USERS_ORDERED_BY_ALIAS_VARIANTS,
            queries::COUNT_USERS_AT_LEAST_AGE_VARIANTS,
            queries::SEARCH_USERS_BY_EITHER_EMAIL_VARIANTS,
            queries::COUNT_USERS_BY_NOTE_VARIANTS,
            queries::SEARCH_USERS_IN_ARRAY_VARIANTS,
        ]
        .concat()
        .into_iter()
        .collect::<std::collections::HashSet<_>>()
        .len()
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

    #[test_context(SqlxPgContext)]
    #[tokio::test]
    async fn warms_every_pooled_connection_and_reuses_prepared_statements(ctx: &mut SqlxPgContext) {
        let pool = warmed(ctx).await;
        let mut first = pool.acquire().await.unwrap();
        let mut second = pool.acquire().await.unwrap();
        // Shared texts, all with matching bind types: the three `COUNT(*)` queries collapse to
        // one all-off text, and `SearchUsersByEitherEmail` renders the all-off and the
        // `email LIKE $1` texts of `SearchUsersByPattern`.
        let warmed = distinct_variants();
        assert_eq!(warmed, queries::DYNFILTER_VARIANT_COUNT - 4);
        for conn in [&mut *first, &mut *second] {
            assert_eq!(conn.cached_statements_size(), warmed);
        }
        let statements = queries::SEARCH_USERS_BY_PATTERN_VARIANTS;
        assert_eq!(statements.len(), 4);
        let server_before = server_statements(&mut first).await;
        for text in statements {
            let prepared: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM pg_prepared_statements WHERE statement = $1)",
            )
            .bind(text)
            .persistent(false)
            .fetch_one(&mut *first)
            .await
            .unwrap();
            assert!(prepared, "{text:?} is not prepared on the server");
        }

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
        assert_eq!(server_statements(&mut first).await, server_before);

        // Warming again is idempotent.
        queries::prepare_dynfilter_variants(&mut first)
            .await
            .unwrap();
        assert_eq!(first.cached_statements_size(), warmed);
        assert_eq!(server_statements(&mut first).await, server_before);

        // A key that differs by one byte would show up as growth, so the checks above can
        // only pass when the runtime renders the warmed text exactly.
        let mismatched = format!("{} ", statements[0]);
        sqlx::Executor::prepare(&mut *first, mismatched.as_str())
            .await
            .unwrap();
        assert_eq!(first.cached_statements_size(), warmed + 1);
        assert_eq!(server_statements(&mut first).await, server_before + 1);
    }

    /// `age` is `INTEGER` but binds as `i64`. Preparing the text untyped lets the server infer
    /// `int4`, which the cached statement then cannot decode from an `i64` bind; the generated
    /// warm-up prepares it with `INT8` instead.
    #[test_context(SqlxPgContext)]
    #[tokio::test]
    async fn typed_warmup_prepares_int4_comparisons_for_i64_binds(ctx: &mut SqlxPgContext) {
        let pool = warmed(ctx).await;
        let mut conn = pool.acquire().await.unwrap();
        let params = || queries::CountUsersAtLeastAgeParams { min_age: Some(1) };
        conn.clear_cached_statements().await.unwrap();
        assert_eq!(
            queries::count_users_at_least_age(&mut *conn, params())
                .await
                .unwrap()
                .total,
            0
        );

        conn.clear_cached_statements().await.unwrap();
        sqlx::Executor::prepare(&mut *conn, queries::COUNT_USERS_AT_LEAST_AGE_VARIANTS[1])
            .await
            .unwrap();
        let error = match queries::count_users_at_least_age(&mut *conn, params()).await {
            Err(error) => error.to_string(),
            Ok(row) => panic!("untyped warm-up should fail, got total {}", row.total),
        };
        assert!(error.contains("incorrect binary data format"), "{error}");

        conn.clear_cached_statements().await.unwrap();
        queries::prepare_count_users_at_least_age(&mut conn)
            .await
            .unwrap();
        let size = conn.cached_statements_size();
        assert_eq!(size, 2);
        for _ in 0..2 {
            assert_eq!(
                queries::count_users_at_least_age(&mut *conn, params())
                    .await
                    .unwrap()
                    .total,
                0
            );
        }
        assert_eq!(conn.cached_statements_size(), size);
    }

    async fn fixture_server_statements(conn: &mut sqlx::PgConnection) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM pg_prepared_statements WHERE statement = ANY($1)")
            .bind(queries::COUNT_USERS_AT_LEAST_AGE_VARIANTS)
            .persistent(false)
            .fetch_one(conn)
            .await
            .unwrap()
    }

    /// sqlx 0.8.6 names every persistent PostgreSQL statement on the server but drops the id
    /// when the local cache is disabled, so capacity `0` is unsupported with the flag on: the
    /// generated warm-up refuses it after at most one leaked statement. A positive but
    /// undersized capacity evicts through `Close` and stays bounded.
    #[test_context(SqlxPgContext)]
    #[tokio::test]
    async fn a_disabled_cache_is_rejected_and_an_undersized_one_stays_bounded(
        ctx: &mut SqlxPgContext,
    ) {
        warmed(ctx).await;
        ctx.rebuild_pool(
            |options| options.statement_cache_capacity(0),
            |pool| pool.max_connections(1),
        )
        .await;
        let mut conn = ctx.pool.acquire().await.unwrap();
        for _ in 0..3 {
            let error = queries::prepare_count_users_at_least_age(&mut conn)
                .await
                .unwrap_err()
                .to_string();
            assert!(error.contains("statement_cache_capacity > 0"), "{error}");
        }
        // Each attempt leaks exactly its first prepare; without the guard both of the
        // fixture's shapes would leak per attempt and this would read 6.
        assert_eq!(fixture_server_statements(&mut conn).await, 3);
        let error = queries::prepare_dynfilter_variants(&mut conn)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("statement_cache_capacity > 0"), "{error}");
        drop(conn);

        let params = || queries::CountUsersAtLeastAgeParams { min_age: Some(1) };
        for (capacity, warm) in [(1, true), (1, false), (2, true)] {
            ctx.rebuild_pool(
                |options| options.statement_cache_capacity(capacity),
                |pool| {
                    let pool = pool.max_connections(1);
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
            let mut conn = ctx.pool.acquire().await.unwrap();
            assert!(conn.cached_statements_size() <= capacity);
            for _ in 0..3 {
                queries::prepare_count_users_at_least_age(&mut conn)
                    .await
                    .unwrap();
                for _ in 0..3 {
                    assert_eq!(
                        queries::count_users_at_least_age(&mut *conn, params())
                            .await
                            .unwrap()
                            .total,
                        0
                    );
                    queries::count_users_at_least_age(&mut *conn, Default::default())
                        .await
                        .unwrap();
                }
                for params in pattern_states() {
                    queries::search_users_by_pattern(&mut *conn, params)
                        .await
                        .unwrap();
                }
            }
            assert!(
                fixture_server_statements(&mut conn).await <= capacity as i64,
                "capacity {capacity} warm {warm}"
            );
            assert!(conn.cached_statements_size() <= capacity);
        }
    }

    /// Server-side identity and execution counters for the fixture's prepared statements on
    /// this connection: `(name, generic_plans + custom_plans)` per variant text.
    async fn plan_counters(conn: &mut sqlx::PgConnection) -> Vec<(String, i64)> {
        let rows: Vec<(String, String, i64)> = sqlx::query_as(
            "SELECT name, statement, generic_plans + custom_plans FROM pg_prepared_statements WHERE statement = ANY($1) ORDER BY statement",
        )
        .bind(queries::SEARCH_USERS_BY_PATTERN_VARIANTS)
        .persistent(false)
        .fetch_all(conn)
        .await
        .unwrap();
        rows.into_iter()
            .map(|(name, _, plans)| (name, plans))
            .collect()
    }

    /// Reuse proven by identity: the warmed statements keep their server names and their plan
    /// counters rise with every generated call, so execution went through them rather than
    /// through a transient unnamed statement.
    #[test_context(SqlxPgContext)]
    #[tokio::test]
    async fn executes_through_the_warmed_server_statements(ctx: &mut SqlxPgContext) {
        let pool = warmed(ctx).await;
        let mut conn = pool.acquire().await.unwrap();
        let before = plan_counters(&mut conn).await;
        assert_eq!(before.len(), 4);
        for round in 1..=2 {
            for params in pattern_states() {
                queries::search_users_by_pattern(&mut *conn, params)
                    .await
                    .unwrap();
            }
            let after = plan_counters(&mut conn).await;
            for ((name, plans), (was, had)) in after.iter().zip(&before) {
                assert_eq!(name, was);
                assert_eq!(*plans, had + round, "{name}");
            }
        }
        // Two source parameters of one type collapse to one text: the shared statement serves
        // both, and each call binds its own value.
        let either = |first, second| queries::SearchUsersByEitherEmailParams { first, second };
        let ids = |users: Vec<queries::SearchUsersByEitherEmailRow>| {
            users.into_iter().map(|user| user.id).collect::<Vec<_>>()
        };
        assert_eq!(queries::SEARCH_USERS_BY_EITHER_EMAIL_VARIANTS.len(), 3);
        let size = conn.cached_statements_size();
        assert_eq!(
            ids(
                queries::search_users_by_either_email(&mut *conn, either(Some("a%"), None))
                    .await
                    .unwrap()
            ),
            [1]
        );
        assert_eq!(
            ids(
                queries::search_users_by_either_email(&mut *conn, either(None, Some("b%")))
                    .await
                    .unwrap()
            ),
            [2]
        );
        assert_eq!(
            ids(
                queries::search_users_by_either_email(&mut *conn, either(Some("a%"), Some("c%")))
                    .await
                    .unwrap()
            ),
            [1, 3]
        );
        assert_eq!(conn.cached_statements_size(), size);
        assert_eq!(
            queries::count_users_by_note(
                &mut *conn,
                queries::CountUsersByNoteParams {
                    note: Some(String::new())
                }
            )
            .await
            .unwrap()
            .total,
            3
        );
    }

    /// The driver behavior behind the shared-text rule: sqlx consults the cache before it
    /// honours `persistent(false)`, so a "skipped" query still binds to whatever another query
    /// prepared for the same text.
    #[test_context(SqlxPgContext)]
    #[tokio::test]
    async fn persistent_false_still_reuses_a_cached_statement(ctx: &mut SqlxPgContext) {
        const SQL: &str = "SELECT 1 WHERE 1::int4 = $1";
        async fn run(conn: &mut sqlx::PgConnection) -> Result<(), sqlx::Error> {
            sqlx::query(SQL)
                .bind(1i32)
                .persistent(false)
                .execute(conn)
                .await
                .map(drop)
        }
        let mut conn = ctx.pool.acquire().await.unwrap();
        run(&mut conn).await.unwrap();
        sqlx::Executor::prepare_with(
            &mut *conn,
            SQL,
            &[<i64 as sqlx::Type<sqlx::Postgres>>::type_info()],
        )
        .await
        .unwrap();
        assert!(run(&mut conn).await.is_err());
        conn.clear_cached_statements().await.unwrap();
        run(&mut conn).await.unwrap();
    }

    /// Release-mode timing. `uncached` runs the generated cache-skipped twin of the fixture
    /// (same SQL, `persistent(false)`), `first use` relies on caching without a warm-up, and
    /// `warmed` runs the aggregate warm-up on connection creation. Migration and seeding happen
    /// before any timer starts; the workload cycles the fixture's four shapes on one connection.
    /// Repeat with `cargo test --release -p dynamic-filter-sqlx-postgres -- --ignored --nocapture`; the recorded
    /// medians live in `examples/dynamic-filter/PREPARED_BENCHMARKS.md`.
    #[test_context(SqlxPgContext)]
    #[tokio::test]
    #[ignore]
    async fn measures_prepared_statement_reuse(ctx: &mut SqlxPgContext) {
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
                conn: &mut sqlx::PgConnection,
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
            "dynamic-filter-sqlx-postgres: 1 connection, warmed shapes {} (when warmed), workload shapes 4, rounds {ROUNDS}\n{}",
            queries::DYNFILTER_VARIANT_COUNT,
            report.join("\n")
        );
    }

    /// PostgreSQL brackets are expressions: the array constructor's parameters bind in every
    /// state, and the conditional email bind takes the number after them.
    #[test_context(SqlxPgContext)]
    #[tokio::test]
    async fn binds_array_constructor_parameters_in_every_state(ctx: &mut SqlxPgContext) {
        let pool = warmed(ctx).await;
        let params = |email| queries::SearchUsersInArrayParams {
            first_id: 1,
            second_id: 2,
            email,
        };

        let users = queries::search_users_in_array(pool, params(None))
            .await
            .unwrap();
        assert_eq!(users.iter().map(|user| user.id).collect::<Vec<_>>(), [1, 2]);

        let users = queries::search_users_in_array(pool, params(Some("bob@example.com")))
            .await
            .unwrap();
        assert_eq!(users.iter().map(|user| user.id).collect::<Vec<_>>(), [2]);
    }
}

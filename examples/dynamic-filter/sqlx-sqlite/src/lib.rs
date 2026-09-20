#[allow(dead_code)]
mod queries;

#[cfg(test)]
mod tests {
    use super::queries;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn database_url() -> String {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "sqlc-gen-rust-dynfilter-{}-{}.db",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        format!("sqlite://{}?mode=rwc", path.display())
    }

    /// The application lifecycle: migrate a fresh file-backed database through a bootstrap
    /// pool, then open the pool every test uses with `capacity` cache entries and a warm-up on
    /// each new connection.
    async fn hooked_pool(url: &str, capacity: usize, warm: bool) -> sqlx::SqlitePool {
        use std::str::FromStr as _;
        let options = sqlx::sqlite::SqliteConnectOptions::from_str(url)
            .unwrap()
            .statement_cache_capacity(capacity);
        let pool = sqlx::sqlite::SqlitePoolOptions::new();
        let pool = if warm {
            pool.after_connect(|conn, _| Box::pin(queries::prepare_dynfilter_variants(conn)))
        } else {
            pool
        };
        pool.connect_with(options).await.unwrap()
    }

    async fn pool() -> sqlx::SqlitePool {
        let url = database_url();
        let bootstrap = sqlx::SqlitePool::connect(&url).await.unwrap();
        migrate(&bootstrap).await;
        bootstrap.close().await;
        hooked_pool(&url, queries::DYNFILTER_VARIANT_COUNT + 16, true).await
    }

    async fn migrate(pool: &sqlx::SqlitePool) {
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

    fn params() -> queries::SearchUsersParams<'static> {
        queries::SearchUsersParams {
            row_limit: 100,
            ..Default::default()
        }
    }

    fn count_users_params() -> queries::CountUsersParams<'static> {
        Default::default()
    }

    #[tokio::test]
    async fn searches_without_filters() {
        let pool = pool().await;

        let users = queries::search_users(&pool, params()).await.unwrap();
        assert_eq!(users.len(), 3);
    }

    #[tokio::test]
    async fn applies_each_scalar_filter() {
        let pool = pool().await;

        let users = queries::search_users(
            &pool,
            queries::SearchUsersParams {
                email: Some("alice@example.com"),
                ..params()
            },
        )
        .await
        .unwrap();
        assert_eq!(users.iter().map(|user| user.id).collect::<Vec<_>>(), [1]);

        let users = queries::search_users(
            &pool,
            queries::SearchUsersParams {
                phone: Some("222"),
                ..params()
            },
        )
        .await
        .unwrap();
        assert_eq!(users.iter().map(|user| user.id).collect::<Vec<_>>(), [2]);
    }

    #[tokio::test]
    async fn distinguishes_none_empty_and_populated_slices() {
        let pool = pool().await;

        assert_eq!(
            queries::search_users(&pool, params()).await.unwrap().len(),
            3
        );

        let empty = [];
        let users = queries::search_users(
            &pool,
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
            &pool,
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

    #[tokio::test]
    async fn toggles_the_orders_block_and_its_filter() {
        let pool = pool().await;

        let users = queries::search_users(
            &pool,
            queries::SearchUsersParams {
                has_orders: true,
                ..params()
            },
        )
        .await
        .unwrap();
        assert_eq!(users.iter().map(|user| user.id).collect::<Vec<_>>(), [1, 2]);

        let users = queries::search_users(
            &pool,
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

    #[tokio::test]
    async fn selects_each_sort_preset() {
        let pool = pool().await;

        let asc = queries::search_users(
            &pool,
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
            &pool,
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
            &pool,
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

        let by_default = queries::search_users(&pool, params()).await.unwrap();
        assert_eq!(
            by_default.iter().map(|user| user.id).collect::<Vec<_>>(),
            [1, 2, 3]
        );
    }

    #[tokio::test]
    async fn counts_users_with_dynamic_filters() {
        let pool = pool().await;

        assert_eq!(
            queries::count_users(&pool, count_users_params())
                .await
                .unwrap()
                .total,
            3
        );
        assert_eq!(
            queries::count_users(
                &pool,
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
                &pool,
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
                &pool,
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
            queries::count_users_opt(&pool, count_users_params())
                .await
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn touches_users_with_dynamic_filters() {
        let pool = pool().await;

        assert_eq!(
            queries::touch_users(&pool, Default::default())
                .await
                .unwrap(),
            3
        );
        assert_eq!(
            queries::touch_users(
                &pool,
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
                &pool,
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
                &pool,
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

    async fn user_ids(
        pool: &sqlx::SqlitePool,
        params: queries::SearchUsersByEmailsParams<'_>,
    ) -> Vec<i64> {
        queries::search_users_by_emails(pool, params)
            .await
            .unwrap()
            .iter()
            .map(|user| user.id)
            .collect()
    }

    #[tokio::test]
    async fn filters_by_owned_string_slices_and_repeated_scalars() {
        let pool = pool().await;
        let emails = [
            "alice@example.com".to_string(),
            "carol@example.com".to_string(),
        ];
        let params = |emails, contact| queries::SearchUsersByEmailsParams { emails, contact };

        assert_eq!(user_ids(&pool, Default::default()).await, [1, 2, 3]);
        assert_eq!(user_ids(&pool, params(Some(&emails), None)).await, [1, 3]);
        assert_eq!(user_ids(&pool, params(None, Some("222"))).await, [2]);
        assert_eq!(
            user_ids(&pool, params(None, Some("bob@example.com"))).await,
            [2]
        );
        assert_eq!(
            user_ids(&pool, params(Some(&emails), Some("111"))).await,
            [1]
        );
        assert!(user_ids(&pool, params(Some(&[]), None)).await.is_empty());
    }

    #[tokio::test]
    async fn combines_where_and_order_switches_with_a_flag_and_a_filter() {
        use queries::{ListUsersByScopeOrder as Order, ListUsersByScopeScope as Scope};
        let pool = pool().await;
        sqlx::query("INSERT INTO users (id, email, phone) VALUES (4, 'dave@example.com', '')")
            .execute(&pool)
            .await
            .unwrap();
        let list = |scope, order, min_id, with_phone| {
            let pool = pool.clone();
            async move {
                queries::list_users_by_scope(
                    &pool,
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
            }
        };

        assert_eq!(Order::default(), Order::Oldest);
        assert_eq!(Scope::default(), Scope::Everyone);
        let by_default = queries::list_users_by_scope(&pool, Default::default())
            .await
            .unwrap();
        assert_eq!(
            by_default.iter().map(|user| user.id).collect::<Vec<_>>(),
            [1, 2, 3, 4]
        );
        for (scope, everyone) in [
            (Scope::Everyone, vec![1, 2, 3, 4]),
            (Scope::WithOrders, vec![1, 2]),
            (Scope::WithoutOrders, vec![3, 4]),
        ] {
            for (order, expected) in [
                (Order::Oldest, everyone.clone()),
                (Order::Newest, everyone.iter().rev().copied().collect()),
            ] {
                assert_eq!(list(scope, order, None, false).await, expected);
                let with_phone = expected
                    .iter()
                    .copied()
                    .filter(|id| *id != 4)
                    .collect::<Vec<_>>();
                assert_eq!(list(scope, order, None, true).await, with_phone);
                let from_two = expected
                    .iter()
                    .copied()
                    .filter(|id| *id >= 2)
                    .collect::<Vec<_>>();
                assert_eq!(list(scope, order, Some(2), false).await, from_two);
            }
        }
    }

    #[tokio::test]
    async fn switches_inside_a_flag_guarded_block_follow_the_flag() {
        use queries::CountUsersWithOrdersOrderAge as Age;
        let pool = pool().await;
        let count = |has_orders, order_age| {
            let pool = pool.clone();
            async move {
                queries::count_users_with_orders(
                    &pool,
                    queries::CountUsersWithOrdersParams {
                        has_orders,
                        order_age,
                    },
                )
                .await
                .unwrap()
                .total
            }
        };

        assert_eq!(count(false, Age::AnyAge).await, 3);
        assert_eq!(count(false, Age::Recent).await, 3);
        assert_eq!(count(true, Age::AnyAge).await, 2);
        assert_eq!(count(true, Age::Recent).await, 1);
        assert_eq!(
            queries::count_users_with_orders(&pool, Default::default())
                .await
                .unwrap()
                .total,
            3
        );
    }

    #[tokio::test]
    async fn dml_with_a_where_switch_and_no_parameters() {
        use queries::ClearPhonesTarget as Target;
        let pool = pool().await;

        assert_eq!(
            queries::clear_phones(
                &pool,
                queries::ClearPhonesParams {
                    target: Target::WithOrders
                }
            )
            .await
            .unwrap(),
            2
        );
        assert_eq!(
            queries::clear_phones(&pool, Default::default())
                .await
                .unwrap(),
            1
        );
    }

    #[tokio::test]
    async fn gates_a_join_with_its_bind_and_ordering() {
        let pool = pool().await;
        sqlx::raw_sql("INSERT INTO users (id, email, phone) VALUES (4, 'dave@example.com', ''), (5, 'erin@example.com', '555'); INSERT INTO orders (id, user_id, created_at) VALUES (3, 4, '2025-06-01'), (4, 1, '2025-02-01');").execute(&pool).await.unwrap();
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
                &pool,
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

    #[tokio::test]
    async fn gates_a_derived_table_join_and_its_unqualified_columns() {
        let pool = pool().await;
        for (with_orders, expected) in [(false, vec![1, 2, 3]), (true, vec![2])] {
            let users = queries::search_users_by_last_order(
                &pool,
                queries::SearchUsersByLastOrderParams { with_orders },
            )
            .await
            .unwrap();
            assert_eq!(
                users.iter().map(|user| user.id).collect::<Vec<_>>(),
                expected
            );
        }
    }

    #[tokio::test]
    async fn gates_a_left_join_and_its_null_extension() {
        let pool = pool().await;
        for (check_orders, expected) in [(false, vec![1, 2, 3]), (true, vec![3])] {
            let users = queries::list_users_without_orders(
                &pool,
                queries::ListUsersWithoutOrdersParams { check_orders },
            )
            .await
            .unwrap();
            assert_eq!(
                users.iter().map(|user| user.id).collect::<Vec<_>>(),
                expected
            );
        }
    }

    #[tokio::test]
    async fn gates_or_operands_with_a_false_fallback() {
        let pool = pool().await;
        for (email_pattern, phone_pattern, expected) in [
            (None, None, vec![]),
            (Some("alice%"), None, vec![1]),
            (None, Some("%3"), vec![3]),
            (Some("alice%"), Some("222"), vec![1, 2]),
        ] {
            let users = queries::search_users_by_pattern(
                &pool,
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
            queries::COUNT_USERS_WITH_ORDERS_VARIANTS,
            queries::SEARCH_USERS_WITH_ORDERS_VARIANTS,
            queries::SEARCH_USERS_BY_PATTERN_VARIANTS,
            queries::SEARCH_USERS_BY_LAST_ORDER_VARIANTS,
            queries::LIST_USERS_WITHOUT_ORDERS_VARIANTS,
            queries::LIST_USERS_HAVING_ALIAS_VARIANTS,
            queries::COUNT_USERS_WITH_PHONED_ORDERS_VARIANTS,
            queries::COUNT_PHONED_USERS_VARIANTS,
        ]
        .concat()
        .into_iter()
        .collect::<std::collections::HashSet<_>>()
        .len()
    }

    #[tokio::test]
    async fn warms_every_pooled_connection_and_reuses_prepared_statements() {
        use sqlx::Connection as _;
        let pool = pool().await;
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
        // A flag-only query has no SQL parameters; both of its states are warmed and reused.
        for with_phone in [false, true] {
            let total = queries::count_phoned_users(
                &mut *first,
                queries::CountPhonedUsersParams { with_phone },
            )
            .await
            .unwrap()
            .total;
            assert_eq!(total, 3);
        }
        assert_eq!(first.cached_statements_size(), warmed);
        // `ClearPhones` is enumerated but listed in `dynfilters.prepared_skip`.
        queries::clear_phones(&mut *first, queries::ClearPhonesParams::default())
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

    #[tokio::test]
    async fn an_undersized_or_disabled_cache_still_executes() {
        use sqlx::Connection as _;
        for capacity in [0, 2] {
            let url = database_url();
            let bootstrap = sqlx::SqlitePool::connect(&url).await.unwrap();
            migrate(&bootstrap).await;
            bootstrap.close().await;
            let pool = hooked_pool(&url, capacity, true).await;
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
    /// Repeat with `cargo test --release -p dynamic-filter-sqlx-sqlite -- --ignored --nocapture`; the recorded
    /// medians live in `examples/dynamic-filter/PREPARED_BENCHMARKS.md`.

    #[tokio::test]
    #[ignore]
    async fn measures_prepared_statement_reuse() {
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
        let url = database_url();
        let bootstrap = sqlx::SqlitePool::connect(&url).await.unwrap();
        migrate(&bootstrap).await;
        bootstrap.close().await;
        let mut report = Vec::new();
        for (label, uncached, warm) in [
            ("uncached (skipped twin)", true, false),
            ("cache on first use", false, false),
            ("warmed", false, true),
        ] {
            let started = std::time::Instant::now();
            let pool = hooked_pool(&url, CAPACITY, warm).await;
            let mut conn = pool.acquire().await.unwrap();
            let connect = started.elapsed();
            async fn run(
                conn: &mut sqlx::SqliteConnection,
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
            "dynamic-filter-sqlx-sqlite: 1 connection, warmed shapes {} (when warmed), workload shapes 4, rounds {ROUNDS}\n{}",
            queries::DYNFILTER_VARIANT_COUNT,
            report.join("\n")
        );
    }
}

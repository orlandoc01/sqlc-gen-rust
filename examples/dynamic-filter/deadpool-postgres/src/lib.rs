#[allow(dead_code)]
mod queries;

#[cfg(test)]
#[path = "tests/dynamic_stream.rs"]
mod dynamic_stream;

#[cfg(test)]
mod tests {
    use futures_util::TryStreamExt as _;
    use test_context::test_context;
    use test_utils::PgDeadpoolContext;

    use super::queries;

    async fn migrate(client: &deadpool_postgres::Client) {
        client
            .batch_execute(include_str!("../schema.sql"))
            .await
            .unwrap();
        client.batch_execute("INSERT INTO users (id, email, phone, profile) VALUES (1, 'alice@example.com', '111', '{\"tier\":\"gold\"}'), (2, 'bob@example.com', '222', '{\"tier\":\"silver\"}'), (3, 'carol@example.com', '333', '{\"tier\":\"gold\"}'); INSERT INTO orders (id, user_id, created_at) VALUES (1, 1, '2024-01-01'), (2, 2, '2025-01-01');").await.unwrap();
    }

    pub(crate) fn warmup_hook() -> deadpool_postgres::Hook {
        deadpool_postgres::Hook::async_fn(|client, _| {
            Box::pin(async move {
                queries::prepare_dynfilter_variants(client)
                    .await
                    .map_err(deadpool_postgres::HookError::Backend)
            })
        })
    }

    /// The application lifecycle: migrate through a plain client, then build the pool every
    /// test uses with a `post_create` hook that warms each new connection.
    async fn warmed(ctx: &mut PgDeadpoolContext) -> deadpool_postgres::Client {
        migrate(&ctx.pool.get().await.unwrap()).await;
        ctx.rebuild_pool(|builder| builder.post_create(warmup_hook()));
        ctx.pool.get().await.unwrap()
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

    #[test_context(PgDeadpoolContext)]
    #[tokio::test]
    async fn searches_without_filters(ctx: &mut PgDeadpoolContext) {
        let client = warmed(ctx).await;
        assert_eq!(
            queries::search_users(&client, params())
                .await
                .unwrap()
                .len(),
            3
        );
        let statement = queries::prepare_list_all_users(&client).await.unwrap();
        assert_eq!(
            queries::list_all_users_with(&client, &statement)
                .await
                .unwrap()
                .len(),
            3
        );
        let stream = queries::list_all_users_stream(&client).await.unwrap();
        futures_util::pin_mut!(stream);
        assert_eq!(stream.try_collect::<Vec<_>>().await.unwrap().len(), 3);
    }

    #[test_context(PgDeadpoolContext)]
    #[tokio::test]
    async fn applies_each_scalar_filter(ctx: &mut PgDeadpoolContext) {
        let client = warmed(ctx).await;
        let users = queries::search_users(
            &client,
            queries::SearchUsersParams {
                email: Some("alice@example.com"),
                ..params()
            },
        )
        .await
        .unwrap();
        assert_eq!(users.iter().map(|user| user.id).collect::<Vec<_>>(), [1]);
        let users = queries::search_users(
            &client,
            queries::SearchUsersParams {
                phone: Some("222"),
                ..params()
            },
        )
        .await
        .unwrap();
        assert_eq!(users.iter().map(|user| user.id).collect::<Vec<_>>(), [2]);
    }

    #[test_context(PgDeadpoolContext)]
    #[tokio::test]
    async fn distinguishes_none_empty_and_populated_slices(ctx: &mut PgDeadpoolContext) {
        let client = warmed(ctx).await;
        assert_eq!(
            queries::search_users(&client, params())
                .await
                .unwrap()
                .len(),
            3
        );
        let empty = [];
        let users = queries::search_users(
            &client,
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
            &client,
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

    #[test_context(PgDeadpoolContext)]
    #[tokio::test]
    async fn toggles_the_orders_block_and_its_filter(ctx: &mut PgDeadpoolContext) {
        let client = warmed(ctx).await;
        let users = queries::search_users(
            &client,
            queries::SearchUsersParams {
                has_orders: true,
                ..params()
            },
        )
        .await
        .unwrap();
        assert_eq!(users.iter().map(|user| user.id).collect::<Vec<_>>(), [1, 2]);
        let users = queries::search_users(
            &client,
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

    #[test_context(PgDeadpoolContext)]
    #[tokio::test]
    async fn selects_each_sort_preset(ctx: &mut PgDeadpoolContext) {
        let client = warmed(ctx).await;
        let asc = queries::search_users(
            &client,
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
            &client,
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
            &client,
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
        let by_default = queries::search_users(&client, params()).await.unwrap();
        assert_eq!(
            by_default.iter().map(|user| user.id).collect::<Vec<_>>(),
            [1, 2, 3]
        );
    }

    #[test_context(PgDeadpoolContext)]
    #[tokio::test]
    async fn counts_users_with_dynamic_filters(ctx: &mut PgDeadpoolContext) {
        let client = warmed(ctx).await;
        assert_eq!(
            queries::count_users(&client, count_users_params())
                .await
                .unwrap()
                .total,
            3
        );
        assert_eq!(
            queries::count_users(
                &client,
                queries::CountUsersParams {
                    email: Some("alice@example.com"),
                    ..count_users_params()
                }
            )
            .await
            .unwrap()
            .total,
            1
        );
        let empty = [];
        assert_eq!(
            queries::count_users(
                &client,
                queries::CountUsersParams {
                    ids: Some(&empty),
                    ..count_users_params()
                }
            )
            .await
            .unwrap()
            .total,
            0
        );
        assert_eq!(
            queries::count_users(
                &client,
                queries::CountUsersParams {
                    ids: None,
                    ..count_users_params()
                }
            )
            .await
            .unwrap()
            .total,
            3
        );
        assert!(
            queries::count_users_opt(&client, count_users_params())
                .await
                .unwrap()
                .is_some()
        );
    }

    #[test_context(PgDeadpoolContext)]
    #[tokio::test]
    async fn touches_users_with_dynamic_filters(ctx: &mut PgDeadpoolContext) {
        let client = warmed(ctx).await;
        assert_eq!(
            queries::touch_users(&client, Default::default())
                .await
                .unwrap(),
            3
        );
        assert_eq!(
            queries::touch_users(
                &client,
                queries::TouchUsersParams {
                    email: Some("alice@example.com"),
                    ..Default::default()
                }
            )
            .await
            .unwrap(),
            1
        );
        let empty = [];
        assert_eq!(
            queries::touch_users(
                &client,
                queries::TouchUsersParams {
                    ids: Some(&empty),
                    ..Default::default()
                }
            )
            .await
            .unwrap(),
            0
        );
        assert_eq!(
            queries::touch_users(
                &client,
                queries::TouchUsersParams {
                    ids: None,
                    ..Default::default()
                }
            )
            .await
            .unwrap(),
            3
        );
    }

    #[test_context(PgDeadpoolContext)]
    #[tokio::test]
    async fn gates_a_join_with_its_bind_and_ordering(ctx: &mut PgDeadpoolContext) {
        let client = warmed(ctx).await;
        client.batch_execute("INSERT INTO users (id, email, phone, profile) VALUES (4, 'dave@example.com', '', '{}'), (5, 'erin@example.com', '555', '{}'); INSERT INTO orders (id, user_id, created_at) VALUES (3, 4, '2025-06-01'), (4, 1, '2025-02-01');").await.unwrap();
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
                &client,
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

    #[test_context(PgDeadpoolContext)]
    #[tokio::test]
    async fn gates_or_operands_with_a_false_fallback(ctx: &mut PgDeadpoolContext) {
        let client = warmed(ctx).await;
        for (email_pattern, phone_pattern, expected) in [
            (None, None, vec![]),
            (Some("alice%"), None, vec![1]),
            (None, Some("%3"), vec![3]),
            (Some("alice%"), Some("222"), vec![1, 2]),
        ] {
            let users = queries::search_users_by_pattern(
                &client,
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
            queries::SEARCH_USERS_VARIANTS,
            queries::COUNT_USERS_VARIANTS,
            queries::TOUCH_USERS_VARIANTS,
            queries::SEARCH_USERS_BY_PROFILE_VARIANTS,
            queries::SET_USER_PHONE_VARIANTS,
            queries::GET_USER_BY_EMAIL_VARIANTS,
            queries::UPDATE_USER_EMAIL_VARIANTS,
            queries::LIST_USERS_BY_SCOPE_VARIANTS,
            queries::SEARCH_USERS_WITH_ORDERS_VARIANTS,
            queries::SEARCH_USERS_BY_PATTERN_VARIANTS,
        ]
        .concat()
        .into_iter()
        .collect::<std::collections::HashSet<_>>()
        .len()
    }

    async fn plan_counters(client: &impl deadpool_postgres::GenericClient) -> Vec<(String, i64)> {
        client
            .query(
                "SELECT name, generic_plans + custom_plans FROM pg_prepared_statements WHERE statement = ANY($1) ORDER BY statement",
                &[&queries::SEARCH_USERS_BY_PATTERN_VARIANTS],
            )
            .await
            .unwrap()
            .iter()
            .map(|row| (row.get(0), row.get(1)))
            .collect()
    }

    /// Reuse proven by identity: the warmed statements keep their server names and their plan
    /// counters rise with every generated call, inside and outside a transaction.
    #[test_context(PgDeadpoolContext)]
    #[tokio::test]
    async fn executes_through_the_warmed_server_statements(ctx: &mut PgDeadpoolContext) {
        let mut client = warmed(ctx).await;
        let before = plan_counters(&client).await;
        assert_eq!(before.len(), 4);
        for params in pattern_states() {
            queries::search_users_by_pattern(&client, params)
                .await
                .unwrap();
        }
        let tx = client.transaction().await.unwrap();
        for params in pattern_states() {
            queries::search_users_by_pattern(&tx, params).await.unwrap();
        }
        let inside = plan_counters(&tx).await;
        tx.commit().await.unwrap();
        for ((name, plans), (was, had)) in inside.iter().zip(&before) {
            assert_eq!(name, was);
            assert_eq!(*plans, had + 2, "{name}");
        }
    }

    async fn server_statements(client: &deadpool_postgres::Client) -> i64 {
        client
            .query_one("SELECT count(*) FROM pg_prepared_statements", &[])
            .await
            .unwrap()
            .get(0)
    }

    #[test_context(PgDeadpoolContext)]
    #[tokio::test]
    async fn warms_every_pooled_connection_and_reuses_prepared_statements(
        ctx: &mut PgDeadpoolContext,
    ) {
        let first = warmed(ctx).await;
        let second = ctx.pool.get().await.unwrap();
        // The count sums per-query variants; two pairs of zero-bind shapes share a text.
        let warmed = distinct_variants();
        assert_eq!(warmed, queries::DYNFILTER_VARIANT_COUNT - 2);
        for client in [&first, &second] {
            assert_eq!(client.statement_cache.size(), warmed);
        }
        let server_before = server_statements(&first).await;
        for text in queries::SEARCH_USERS_BY_PATTERN_VARIANTS {
            let prepared: bool = first
                .query_one(
                    "SELECT EXISTS (SELECT 1 FROM pg_prepared_statements WHERE statement = $1)",
                    &[text],
                )
                .await
                .unwrap()
                .get(0);
            assert!(prepared, "{text:?} is not prepared on the server");
        }

        for _ in 0..2 {
            for params in pattern_states() {
                queries::search_users_by_pattern(&first, params.clone())
                    .await
                    .unwrap();
                queries::search_users_by_pattern(&second, params)
                    .await
                    .unwrap();
            }
        }
        assert_eq!(first.statement_cache.size(), warmed);
        assert_eq!(server_statements(&first).await, server_before);

        queries::prepare_dynfilter_variants(&first).await.unwrap();
        assert_eq!(first.statement_cache.size(), warmed);
        assert_eq!(server_statements(&first).await, server_before);

        let mut first = first;
        let tx = first.transaction().await.unwrap();
        assert_eq!(
            queries::search_users_by_pattern(&tx, pattern_states().into_iter().last().unwrap())
                .await
                .unwrap()
                .len(),
            3
        );
        tx.commit().await.unwrap();
        assert_eq!(first.statement_cache.size(), warmed);

        // A key that differs by one byte would show up as growth, so the checks above can
        // only pass when the runtime renders the warmed text exactly.
        let mismatched = format!("{} ", queries::SEARCH_USERS_BY_PATTERN_VARIANTS[0]);
        first.prepare_cached(&mismatched).await.unwrap();
        assert_eq!(first.statement_cache.size(), warmed + 1);
        assert_eq!(server_statements(&first).await, server_before + 1);
    }

    /// Release-mode timing. `uncached` runs the generated cache-skipped twin of the fixture
    /// (same SQL, `sql.as_str()`), `first use` relies on caching without a warm-up, and
    /// `warmed` runs the aggregate warm-up on connection creation. Migration and seeding happen
    /// before any timer starts; the workload cycles the fixture's four shapes on one connection.
    /// Repeat with `cargo test --release -p dynamic-filter-deadpool-postgres -- --ignored --nocapture`; the recorded
    /// medians live in `examples/dynamic-filter/PREPARED_BENCHMARKS.md`.
    #[test_context(PgDeadpoolContext)]
    #[tokio::test]
    #[ignore]
    async fn measures_prepared_statement_reuse(ctx: &mut PgDeadpoolContext) {
        const ROUNDS: usize = 200;
        migrate(&ctx.pool.get().await.unwrap()).await;
        let hot = || queries::SearchUsersByPatternParams {
            email_pattern: Some("a%"),
            phone_pattern: None,
        };
        async fn run(
            client: &deadpool_postgres::Client,
            uncached: bool,
            params: queries::SearchUsersByPatternParams<'static>,
        ) -> usize {
            if uncached {
                queries::search_users_by_pattern_uncached(
                    client,
                    queries::SearchUsersByPatternUncachedParams {
                        email_pattern: params.email_pattern,
                        phone_pattern: params.phone_pattern,
                    },
                )
                .await
                .unwrap()
                .len()
            } else {
                queries::search_users_by_pattern(client, params)
                    .await
                    .unwrap()
                    .len()
            }
        }
        let mut report = Vec::new();
        for (label, uncached, warm) in [
            ("uncached (skipped twin)", true, false),
            ("cache on first use", false, false),
            ("warmed", false, true),
        ] {
            let started = std::time::Instant::now();
            ctx.rebuild_pool(|builder| {
                let builder = builder.max_size(1);
                if warm {
                    builder.post_create(warmup_hook())
                } else {
                    builder
                }
            });
            let client = ctx.pool.get().await.unwrap();
            let connect = started.elapsed();
            let started = std::time::Instant::now();
            run(&client, uncached, hot()).await;
            let first = started.elapsed();
            let started = std::time::Instant::now();
            for _ in 0..ROUNDS {
                run(&client, uncached, hot()).await;
            }
            let hot_query = started.elapsed() / ROUNDS as u32;
            let started = std::time::Instant::now();
            for _ in 0..ROUNDS {
                for params in pattern_states() {
                    run(&client, uncached, params).await;
                }
            }
            let mixed = started.elapsed() / (ROUNDS * 4) as u32;
            report.push(format!(
                "{label:<24} connect+warm {connect:>9.2?} first {first:>9.2?} hot {hot_query:>9.2?}/call mixed {mixed:>9.2?}/call cached {} capacity unbounded",
                client.statement_cache.size()
            ));
        }
        println!(
            "dynamic-filter-deadpool-postgres: 1 connection, warmed shapes {} (when warmed), workload shapes 4, rounds {ROUNDS}\n{}",
            queries::DYNFILTER_VARIANT_COUNT,
            report.join("\n")
        );
    }
}

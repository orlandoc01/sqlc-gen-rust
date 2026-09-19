#[allow(dead_code)]
mod queries;

#[cfg(test)]
mod tests {
    use super::queries;

    /// The application lifecycle: migrate, size the statement cache for every enumerated shape
    /// plus other queries, then warm it before the first query.
    fn connection() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        migrate(&conn);
        conn.set_prepared_statement_cache_capacity(queries::DYNFILTER_VARIANT_COUNT + 16);
        queries::prepare_dynfilter_variants(&conn).unwrap();
        conn
    }

    fn migrate(conn: &rusqlite::Connection) {
        conn.execute_batch(include_str!("../../sqlx-sqlite/schema.sql"))
            .unwrap();
        conn.execute_batch(
            "INSERT INTO users (id, email, phone) VALUES (1, 'alice@example.com', '111'), (2, 'bob@example.com', '222'), (3, 'carol@example.com', '333');
             INSERT INTO orders (id, user_id, created_at) VALUES (1, 1, '2024-01-01'), (2, 2, '2025-01-01');",
        )
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

    #[test]
    fn searches_without_filters() {
        let conn = connection();

        assert_eq!(queries::search_users(&conn, params()).unwrap().len(), 3);
    }

    #[test]
    fn applies_each_scalar_filter() {
        let conn = connection();

        let users = queries::search_users(
            &conn,
            queries::SearchUsersParams {
                email: Some("alice@example.com"),
                ..params()
            },
        )
        .unwrap();
        assert_eq!(users.iter().map(|user| user.id).collect::<Vec<_>>(), [1]);

        let users = queries::search_users(
            &conn,
            queries::SearchUsersParams {
                phone: Some("222"),
                ..params()
            },
        )
        .unwrap();
        assert_eq!(users.iter().map(|user| user.id).collect::<Vec<_>>(), [2]);
    }

    #[test]
    fn distinguishes_none_empty_and_populated_slices() {
        let conn = connection();

        assert_eq!(queries::search_users(&conn, params()).unwrap().len(), 3);
        let empty = [];
        let users = queries::search_users(
            &conn,
            queries::SearchUsersParams {
                ids: Some(&empty),
                ..params()
            },
        )
        .unwrap();
        assert!(users.is_empty());

        let ids = [1, 3];
        let users = queries::search_users(
            &conn,
            queries::SearchUsersParams {
                ids: Some(&ids),
                ..params()
            },
        )
        .unwrap();
        assert_eq!(users.iter().map(|user| user.id).collect::<Vec<_>>(), [1, 3]);
        assert_eq!(queries::dynfilter::nilable(&empty), None);
    }

    #[test]
    fn toggles_the_orders_block_and_its_filter() {
        let conn = connection();

        let users = queries::search_users(
            &conn,
            queries::SearchUsersParams {
                has_orders: true,
                ..params()
            },
        )
        .unwrap();
        assert_eq!(users.iter().map(|user| user.id).collect::<Vec<_>>(), [1, 2]);

        let users = queries::search_users(
            &conn,
            queries::SearchUsersParams {
                has_orders: true,
                orders_since: Some("2025-01-01"),
                ..params()
            },
        )
        .unwrap();
        assert_eq!(users.iter().map(|user| user.id).collect::<Vec<_>>(), [2]);
    }

    #[test]
    fn selects_each_sort_preset() {
        let conn = connection();

        let asc = queries::search_users(
            &conn,
            queries::SearchUsersParams {
                sort: queries::SearchUsersSort::IdAsc,
                ..params()
            },
        )
        .unwrap();
        assert_eq!(
            asc.iter().map(|user| user.id).collect::<Vec<_>>(),
            [1, 2, 3]
        );

        let desc = queries::search_users(
            &conn,
            queries::SearchUsersParams {
                sort: queries::SearchUsersSort::IdDesc,
                ..params()
            },
        )
        .unwrap();
        assert_eq!(
            desc.iter().map(|user| user.id).collect::<Vec<_>>(),
            [3, 2, 1]
        );

        let shortest = queries::search_users(
            &conn,
            queries::SearchUsersParams {
                sort: queries::SearchUsersSort::ShortestEmail,
                ..params()
            },
        )
        .unwrap();
        assert_eq!(
            shortest.iter().map(|user| user.id).collect::<Vec<_>>(),
            [2, 3, 1]
        );

        assert_eq!(
            queries::SearchUsersSort::default(),
            queries::SearchUsersSort::IdAsc
        );

        let by_default = queries::search_users(&conn, params()).unwrap();
        assert_eq!(
            by_default.iter().map(|user| user.id).collect::<Vec<_>>(),
            [1, 2, 3]
        );
    }

    #[test]
    fn counts_users_with_dynamic_filters() {
        let conn = connection();

        assert_eq!(
            queries::count_users(&conn, count_users_params())
                .unwrap()
                .total,
            3
        );
        assert_eq!(
            queries::count_users(
                &conn,
                queries::CountUsersParams {
                    email: Some("alice@example.com"),
                    ..count_users_params()
                },
            )
            .unwrap()
            .total,
            1
        );

        let empty = [];
        assert_eq!(
            queries::count_users(
                &conn,
                queries::CountUsersParams {
                    ids: Some(&empty),
                    ..count_users_params()
                },
            )
            .unwrap()
            .total,
            0
        );
        assert_eq!(
            queries::count_users(
                &conn,
                queries::CountUsersParams {
                    ids: None,
                    ..count_users_params()
                },
            )
            .unwrap()
            .total,
            3
        );
        assert!(
            queries::count_users_opt(&conn, count_users_params())
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn touches_users_with_dynamic_filters() {
        let conn = connection();

        assert_eq!(queries::touch_users(&conn, Default::default()).unwrap(), 3);
        assert_eq!(
            queries::touch_users(
                &conn,
                queries::TouchUsersParams {
                    email: Some("alice@example.com"),
                    ..Default::default()
                },
            )
            .unwrap(),
            1
        );

        let empty = [];
        assert_eq!(
            queries::touch_users(
                &conn,
                queries::TouchUsersParams {
                    ids: Some(&empty),
                    ..Default::default()
                },
            )
            .unwrap(),
            0
        );
        assert_eq!(
            queries::touch_users(
                &conn,
                queries::TouchUsersParams {
                    ids: None,
                    ..Default::default()
                },
            )
            .unwrap(),
            3
        );
    }

    #[test]
    fn filters_by_owned_string_slices_and_repeated_scalars() {
        let conn = connection();
        let ids = |params| {
            queries::search_users_by_emails(&conn, params)
                .unwrap()
                .iter()
                .map(|user| user.id)
                .collect::<Vec<_>>()
        };
        let emails = [
            "alice@example.com".to_string(),
            "carol@example.com".to_string(),
        ];

        assert_eq!(ids(Default::default()), [1, 2, 3]);
        assert_eq!(
            ids(queries::SearchUsersByEmailsParams {
                emails: Some(&emails),
                contact: None,
            }),
            [1, 3]
        );
        assert_eq!(
            ids(queries::SearchUsersByEmailsParams {
                emails: None,
                contact: Some("222"),
            }),
            [2]
        );
        assert_eq!(
            ids(queries::SearchUsersByEmailsParams {
                emails: None,
                contact: Some("bob@example.com"),
            }),
            [2]
        );
        assert_eq!(
            ids(queries::SearchUsersByEmailsParams {
                emails: Some(&emails),
                contact: Some("111"),
            }),
            [1]
        );
        assert!(
            ids(queries::SearchUsersByEmailsParams {
                emails: Some(&[]),
                contact: None,
            })
            .is_empty()
        );
    }

    #[test]
    fn gates_a_join_with_its_bind_and_ordering() {
        let conn = connection();
        conn.execute_batch("INSERT INTO users (id, email, phone) VALUES (4, 'dave@example.com', ''), (5, 'erin@example.com', '555'); INSERT INTO orders (id, user_id, created_at) VALUES (3, 4, '2025-06-01'), (4, 1, '2025-02-01');").unwrap();
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
                &conn,
                queries::SearchUsersWithOrdersParams {
                    orders_since,
                    with_phone,
                },
            )
            .unwrap();
            assert_eq!(
                users.iter().map(|user| user.id).collect::<Vec<_>>(),
                expected
            );
        }
    }

    #[test]
    fn gates_a_derived_table_join_and_its_unqualified_columns() {
        let conn = connection();
        for (with_orders, expected) in [(false, vec![1, 2, 3]), (true, vec![2])] {
            let users = queries::search_users_by_last_order(
                &conn,
                queries::SearchUsersByLastOrderParams { with_orders },
            )
            .unwrap();
            assert_eq!(
                users.iter().map(|user| user.id).collect::<Vec<_>>(),
                expected
            );
        }
    }

    #[test]
    fn gates_a_left_join_and_its_null_extension() {
        let conn = connection();
        for (check_orders, expected) in [(false, vec![1, 2, 3]), (true, vec![3])] {
            let users = queries::list_users_without_orders(
                &conn,
                queries::ListUsersWithoutOrdersParams { check_orders },
            )
            .unwrap();
            assert_eq!(
                users.iter().map(|user| user.id).collect::<Vec<_>>(),
                expected
            );
        }
    }

    #[test]
    fn resolves_an_output_alias_in_having_and_order_by_on_sqlite() {
        let conn = connection();
        for (with_orders, expected) in [(false, vec![1, 2, 3]), (true, vec![1, 2])] {
            let users = queries::list_users_having_alias(
                &conn,
                queries::ListUsersHavingAliasParams { with_orders },
            )
            .unwrap();
            assert_eq!(
                users.iter().map(|user| user.id).collect::<Vec<_>>(),
                expected
            );
        }
    }

    #[test]
    fn gates_a_join_inside_a_subquery_from_a_standalone_annotation() {
        let conn = connection();
        conn.execute_batch(
            "INSERT INTO users (id, email, phone) VALUES (4, 'dave@example.com', ''); INSERT INTO orders (id, user_id, created_at) VALUES (3, 4, '2025-06-01');",
        )
        .unwrap();
        for (with_phone, expected) in [(false, 3), (true, 2)] {
            let total = queries::count_users_with_phoned_orders(
                &conn,
                queries::CountUsersWithPhonedOrdersParams { with_phone },
            )
            .unwrap()
            .total;
            assert_eq!(total, expected);
        }
    }

    #[test]
    fn gates_or_operands_with_a_false_fallback() {
        let conn = connection();
        for (email_pattern, phone_pattern, expected) in [
            (None, None, vec![]),
            (Some("alice%"), None, vec![1]),
            (None, Some("%3"), vec![3]),
            (Some("alice%"), Some("222"), vec![1, 2]),
        ] {
            let users = queries::search_users_by_pattern(
                &conn,
                queries::SearchUsersByPatternParams {
                    email_pattern,
                    phone_pattern,
                },
            )
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

    /// rusqlite exposes no cache size, so reuse is proven by the generator's `prepare_cached`
    /// call path; here every enumerated shape executes through the warmed cache, twice, and the
    /// warm-up is repeatable after a flush.
    #[test]
    fn executes_every_shape_through_the_warmed_cache() {
        let conn = connection();
        assert_eq!(queries::SEARCH_USERS_BY_PATTERN_VARIANTS.len(), 4);
        for _ in 0..2 {
            for params in pattern_states() {
                queries::search_users_by_pattern(&conn, params).unwrap();
            }
        }
        conn.flush_prepared_statement_cache();
        queries::prepare_dynfilter_variants(&conn).unwrap();
        queries::prepare_search_users_by_pattern(&conn).unwrap();
        let ids = [1, 2];
        let users = queries::search_users(
            &conn,
            queries::SearchUsersParams {
                ids: Some(&ids),
                ..params()
            },
        )
        .unwrap();
        assert_eq!(users.len(), 2);

        let mut conn = conn;
        let tx = conn.transaction().unwrap();
        assert_eq!(
            queries::search_users_by_pattern(&tx, pattern_states().into_iter().last().unwrap())
                .unwrap()
                .len(),
            3
        );
        tx.commit().unwrap();
    }

    /// Reuse proven by SQLite's per-statement run counter: the cached statement each variant
    /// text maps to is the one the generated function steps, so its `Run` count rises with
    /// every call and would stay flat if execution prepared a transient statement instead.
    #[test]
    fn executes_through_the_cached_statements() {
        let conn = connection();
        let runs = |conn: &rusqlite::Connection| {
            queries::SEARCH_USERS_BY_PATTERN_VARIANTS
                .iter()
                .map(|sql| {
                    conn.prepare_cached(sql)
                        .unwrap()
                        .get_status(rusqlite::StatementStatus::Run)
                })
                .collect::<Vec<_>>()
        };
        let before = runs(&conn);
        for round in 1..=2 {
            for params in pattern_states() {
                queries::search_users_by_pattern(&conn, params).unwrap();
            }
            let after = runs(&conn);
            assert!(
                after
                    .iter()
                    .zip(&before)
                    .all(|(now, was)| *now == was + round),
                "{before:?} -> {after:?}"
            );
        }
    }

    /// Release-mode timing. `uncached` runs the generated cache-skipped twin of the fixture
    /// (same SQL, `prepare(&sql)`), `first use` relies on caching without a warm-up, and
    /// `warmed` runs the aggregate warm-up on connection creation. Migration and seeding happen
    /// before any timer starts; the workload cycles the fixture's four shapes on one connection.
    /// Repeat with `cargo test --release -p dynamic-filter-rusqlite -- --ignored --nocapture`; the recorded
    /// medians live in `examples/dynamic-filter/PREPARED_BENCHMARKS.md`.
    #[test]
    #[ignore]
    fn measures_prepared_statement_reuse() {
        const ROUNDS: usize = 2000;
        const CAPACITY: usize = 128;
        let hot = || queries::SearchUsersByPatternParams {
            email_pattern: Some("a%"),
            phone_pattern: None,
        };
        fn run(
            conn: &rusqlite::Connection,
            uncached: bool,
            params: queries::SearchUsersByPatternParams<'static>,
        ) -> usize {
            if uncached {
                queries::search_users_by_pattern_uncached(
                    conn,
                    queries::SearchUsersByPatternUncachedParams {
                        email_pattern: params.email_pattern,
                        phone_pattern: params.phone_pattern,
                    },
                )
                .unwrap()
                .len()
            } else {
                queries::search_users_by_pattern(conn, params)
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
            let conn = rusqlite::Connection::open_in_memory().unwrap();
            migrate(&conn);
            let started = std::time::Instant::now();
            conn.set_prepared_statement_cache_capacity(CAPACITY);
            if warm {
                queries::prepare_dynfilter_variants(&conn).unwrap();
            }
            let connect = started.elapsed();
            let started = std::time::Instant::now();
            run(&conn, uncached, hot());
            let first = started.elapsed();
            let started = std::time::Instant::now();
            for _ in 0..ROUNDS {
                run(&conn, uncached, hot());
            }
            let hot_query = started.elapsed() / ROUNDS as u32;
            let started = std::time::Instant::now();
            for _ in 0..ROUNDS {
                for params in pattern_states() {
                    run(&conn, uncached, params);
                }
            }
            let mixed = started.elapsed() / (ROUNDS * 4) as u32;
            report.push(format!(
                "{label:<24} warm {connect:>9.2?} first {first:>9.2?} hot {hot_query:>9.2?}/call mixed {mixed:>9.2?}/call capacity {CAPACITY}"
            ));
        }
        println!(
            "dynamic-filter-rusqlite: 1 connection, warmed shapes {} (when warmed), workload shapes 4, rounds {ROUNDS}\n{}",
            queries::DYNFILTER_VARIANT_COUNT,
            report.join("\n")
        );
    }
}

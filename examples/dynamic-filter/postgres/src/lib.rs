#[allow(dead_code)]
mod queries;

#[cfg(test)]
mod tests {
    use postgres::fallible_iterator::FallibleIterator as _;
    use test_context::test_context;
    use test_utils::PgSyncContext;

    use super::queries;

    fn migrate(client: &mut postgres::Client) {
        client.batch_execute(include_str!("../schema.sql")).unwrap();
        client
            .batch_execute(
                r#"INSERT INTO users (id, email, phone, profile) VALUES
                    (1, 'alice@example.com', '111', '{"tier":"gold"}'),
                    (2, 'bob@example.com', '222', '{"tier":"silver"}'),
                    (3, 'carol@example.com', '333', '{"tier":"gold"}');
                   INSERT INTO orders (id, user_id, created_at) VALUES (1, 1, '2024-01-01'), (2, 2, '2025-01-01');"#,
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

    #[test_context(PgSyncContext)]
    #[test]
    fn searches_without_filters(ctx: &mut PgSyncContext) {
        let client = &mut ctx.client;
        migrate(client);
        assert_eq!(queries::search_users(client, params()).unwrap().len(), 3);

        let statement = queries::prepare_list_all_users(client).unwrap();
        assert_eq!(
            queries::list_all_users_with(client, &statement)
                .unwrap()
                .len(),
            3
        );
        assert_eq!(
            queries::list_all_users_iter(client)
                .unwrap()
                .collect::<Vec<_>>()
                .unwrap()
                .len(),
            3
        );
    }

    #[test_context(PgSyncContext)]
    #[test]
    fn applies_each_scalar_filter(ctx: &mut PgSyncContext) {
        let client = &mut ctx.client;
        migrate(client);
        let users = queries::search_users(
            client,
            queries::SearchUsersParams {
                email: Some("alice@example.com"),
                ..params()
            },
        )
        .unwrap();
        assert_eq!(users.iter().map(|user| user.id).collect::<Vec<_>>(), [1]);
        let users = queries::search_users(
            client,
            queries::SearchUsersParams {
                phone: Some("222"),
                ..params()
            },
        )
        .unwrap();
        assert_eq!(users.iter().map(|user| user.id).collect::<Vec<_>>(), [2]);
    }

    #[test_context(PgSyncContext)]
    #[test]
    fn distinguishes_none_empty_and_populated_slices(ctx: &mut PgSyncContext) {
        let client = &mut ctx.client;
        migrate(client);
        assert_eq!(queries::search_users(client, params()).unwrap().len(), 3);
        let empty = [];
        let users = queries::search_users(
            client,
            queries::SearchUsersParams {
                ids: Some(&empty),
                ..params()
            },
        )
        .unwrap();
        assert!(users.is_empty());
        let ids = [1, 3];
        let users = queries::search_users(
            client,
            queries::SearchUsersParams {
                ids: Some(&ids),
                ..params()
            },
        )
        .unwrap();
        assert_eq!(users.iter().map(|user| user.id).collect::<Vec<_>>(), [1, 3]);
        assert_eq!(queries::dynfilter::nilable(&empty), None);
    }

    #[test_context(PgSyncContext)]
    #[test]
    fn toggles_the_orders_block_and_its_filter(ctx: &mut PgSyncContext) {
        let client = &mut ctx.client;
        migrate(client);
        let users = queries::search_users(
            client,
            queries::SearchUsersParams {
                has_orders: true,
                ..params()
            },
        )
        .unwrap();
        assert_eq!(users.iter().map(|user| user.id).collect::<Vec<_>>(), [1, 2]);
        let users = queries::search_users(
            client,
            queries::SearchUsersParams {
                has_orders: true,
                orders_since: Some("2025-01-01"),
                ..params()
            },
        )
        .unwrap();
        assert_eq!(users.iter().map(|user| user.id).collect::<Vec<_>>(), [2]);
    }

    #[test_context(PgSyncContext)]
    #[test]
    fn selects_each_sort_preset(ctx: &mut PgSyncContext) {
        let client = &mut ctx.client;
        migrate(client);
        let asc = queries::search_users(
            client,
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
            client,
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
            client,
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
        let by_default = queries::search_users(client, params()).unwrap();
        assert_eq!(
            by_default.iter().map(|user| user.id).collect::<Vec<_>>(),
            [1, 2, 3]
        );
    }

    #[test_context(PgSyncContext)]
    #[test]
    fn counts_users_with_dynamic_filters(ctx: &mut PgSyncContext) {
        let client = &mut ctx.client;
        migrate(client);
        assert_eq!(
            queries::count_users(client, count_users_params())
                .unwrap()
                .total,
            3
        );
        assert_eq!(
            queries::count_users(
                client,
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
                client,
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
                client,
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
            queries::count_users_opt(client, count_users_params())
                .unwrap()
                .is_some()
        );
    }

    #[test_context(PgSyncContext)]
    #[test]
    fn touches_users_with_dynamic_filters(ctx: &mut PgSyncContext) {
        let client = &mut ctx.client;
        migrate(client);
        assert_eq!(queries::touch_users(client, Default::default()).unwrap(), 3);
        assert_eq!(
            queries::touch_users(
                client,
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
                client,
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
                client,
                queries::TouchUsersParams {
                    ids: None,
                    ..Default::default()
                },
            )
            .unwrap(),
            3
        );
    }

    #[test_context(PgSyncContext)]
    #[test]
    fn reports_dynamic_one_cardinality(ctx: &mut PgSyncContext) {
        let client = &mut ctx.client;
        migrate(client);

        let missing = queries::GetUserByEmailParams {
            email: Some("missing@example.com"),
        };
        assert!(queries::get_user_by_email(client, missing).is_err());
        assert!(
            queries::get_user_by_email_opt(
                client,
                queries::GetUserByEmailParams {
                    email: Some("missing@example.com"),
                },
            )
            .unwrap()
            .is_none()
        );

        let user = queries::get_user_by_email(
            client,
            queries::GetUserByEmailParams {
                email: Some("alice@example.com"),
            },
        )
        .unwrap();
        assert_eq!(user.id, 1);
        assert_eq!(
            queries::get_user_by_email_opt(
                client,
                queries::GetUserByEmailParams {
                    email: Some("alice@example.com"),
                },
            )
            .unwrap()
            .unwrap()
            .id,
            1
        );

        assert!(queries::get_user_by_email(client, Default::default()).is_err());
        assert!(queries::get_user_by_email_opt(client, Default::default()).is_err());
    }

    #[test_context(PgSyncContext)]
    #[test]
    fn executes_dynamic_mutations_and_propagates_constraints(ctx: &mut PgSyncContext) {
        let client = &mut ctx.client;
        migrate(client);

        queries::update_user_email(
            client,
            queries::UpdateUserEmailParams {
                new_email: "alice-renamed@example.com",
                id: 1,
                email: Some("alice@example.com"),
            },
        )
        .unwrap();
        queries::update_user_email(
            client,
            queries::UpdateUserEmailParams {
                new_email: "bob-renamed@example.com",
                id: 2,
                email: None,
            },
        )
        .unwrap();
        assert!(
            queries::update_user_email(
                client,
                queries::UpdateUserEmailParams {
                    new_email: "alice-renamed@example.com",
                    id: 3,
                    email: None,
                },
            )
            .is_err()
        );
    }

    #[test_context(PgSyncContext)]
    #[test]
    fn gates_a_join_with_its_bind_and_ordering(ctx: &mut PgSyncContext) {
        let client = &mut ctx.client;
        migrate(client);
        client.batch_execute("INSERT INTO users (id, email, phone, profile) VALUES (4, 'dave@example.com', '', '{}'), (5, 'erin@example.com', '555', '{}'); INSERT INTO orders (id, user_id, created_at) VALUES (3, 4, '2025-06-01'), (4, 1, '2025-02-01');").unwrap();
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
                client,
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

    #[test_context(PgSyncContext)]
    #[test]
    fn gates_a_derived_table_join_and_its_unqualified_columns(ctx: &mut PgSyncContext) {
        let client = &mut ctx.client;
        migrate(client);
        for (with_orders, expected) in [(false, vec![1, 2, 3]), (true, vec![2])] {
            let users = queries::search_users_by_last_order(
                client,
                queries::SearchUsersByLastOrderParams { with_orders },
            )
            .unwrap();
            assert_eq!(
                users.iter().map(|user| user.id).collect::<Vec<_>>(),
                expected
            );
        }
    }

    #[test_context(PgSyncContext)]
    #[test]
    fn gates_or_operands_with_a_false_fallback(ctx: &mut PgSyncContext) {
        let client = &mut ctx.client;
        migrate(client);
        for (email_pattern, phone_pattern, expected) in [
            (None, None, vec![]),
            (Some("alice%"), None, vec![1]),
            (None, Some("%3"), vec![3]),
            (Some("alice%"), Some("222"), vec![1, 2]),
        ] {
            let users = queries::search_users_by_pattern(
                client,
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
}

#[cfg(test)]
#[path = "tests/dynamic_iter.rs"]
mod dynamic_iter;

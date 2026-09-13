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
    fn toggles_each_order_by_direction(ctx: &mut PgSyncContext) {
        let client = &mut ctx.client;
        migrate(client);
        let asc = queries::search_users(
            client,
            queries::SearchUsersParams {
                id_asc: true,
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
                id_desc: true,
                ..params()
            },
        )
        .unwrap();
        assert_eq!(
            desc.iter().map(|user| user.id).collect::<Vec<_>>(),
            [3, 2, 1]
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
}

#[cfg(test)]
#[path = "tests/dynamic_iter.rs"]
mod dynamic_iter;

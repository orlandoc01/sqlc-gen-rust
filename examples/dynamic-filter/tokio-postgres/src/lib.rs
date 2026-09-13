#[allow(dead_code)]
mod queries;

#[cfg(test)]
mod tests {
    use futures_util::TryStreamExt as _;
    use test_context::test_context;
    use test_utils::PgTokioContext;

    use super::queries;

    async fn migrate(client: &tokio_postgres::Client) {
        client
            .batch_execute(include_str!("../schema.sql"))
            .await
            .unwrap();
        client
            .batch_execute(
                r#"INSERT INTO users (id, email, phone, profile) VALUES
                    (1, 'alice@example.com', '111', '{"tier":"gold"}'),
                    (2, 'bob@example.com', '222', '{"tier":"silver"}'),
                    (3, 'carol@example.com', '333', '{"tier":"gold"}');
                   INSERT INTO orders (id, user_id, created_at) VALUES (1, 1, '2024-01-01'), (2, 2, '2025-01-01');"#,
            )
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

    #[test_context(PgTokioContext)]
    #[tokio::test]
    async fn searches_without_filters(ctx: &mut PgTokioContext) {
        let client = &ctx.client;
        migrate(client).await;
        assert_eq!(
            queries::search_users(client, params()).await.unwrap().len(),
            3
        );

        let statement = queries::prepare_list_all_users(client).await.unwrap();
        assert_eq!(
            queries::list_all_users_with(client, &statement)
                .await
                .unwrap()
                .len(),
            3
        );
        let stream = queries::list_all_users_stream(client).await.unwrap();
        futures_util::pin_mut!(stream);
        assert_eq!(stream.try_collect::<Vec<_>>().await.unwrap().len(), 3);
    }

    #[test_context(PgTokioContext)]
    #[tokio::test]
    async fn applies_each_scalar_filter(ctx: &mut PgTokioContext) {
        let client = &ctx.client;
        migrate(client).await;
        let users = queries::search_users(
            client,
            queries::SearchUsersParams {
                email: Some("alice@example.com"),
                ..params()
            },
        )
        .await
        .unwrap();
        assert_eq!(users.iter().map(|user| user.id).collect::<Vec<_>>(), [1]);
        let users = queries::search_users(
            client,
            queries::SearchUsersParams {
                phone: Some("222"),
                ..params()
            },
        )
        .await
        .unwrap();
        assert_eq!(users.iter().map(|user| user.id).collect::<Vec<_>>(), [2]);
    }

    #[test_context(PgTokioContext)]
    #[tokio::test]
    async fn distinguishes_none_empty_and_populated_slices(ctx: &mut PgTokioContext) {
        let client = &ctx.client;
        migrate(client).await;
        assert_eq!(
            queries::search_users(client, params()).await.unwrap().len(),
            3
        );
        let empty = [];
        let users = queries::search_users(
            client,
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
            client,
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

    #[test_context(PgTokioContext)]
    #[tokio::test]
    async fn toggles_the_orders_block_and_its_filter(ctx: &mut PgTokioContext) {
        let client = &ctx.client;
        migrate(client).await;
        let users = queries::search_users(
            client,
            queries::SearchUsersParams {
                has_orders: true,
                ..params()
            },
        )
        .await
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
        .await
        .unwrap();
        assert_eq!(users.iter().map(|user| user.id).collect::<Vec<_>>(), [2]);
    }

    #[test_context(PgTokioContext)]
    #[tokio::test]
    async fn toggles_each_order_by_direction(ctx: &mut PgTokioContext) {
        let client = &ctx.client;
        migrate(client).await;
        let asc = queries::search_users(
            client,
            queries::SearchUsersParams {
                id_asc: true,
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
            client,
            queries::SearchUsersParams {
                id_desc: true,
                ..params()
            },
        )
        .await
        .unwrap();
        assert_eq!(
            desc.iter().map(|user| user.id).collect::<Vec<_>>(),
            [3, 2, 1]
        );
    }

    #[test_context(PgTokioContext)]
    #[tokio::test]
    async fn counts_users_with_dynamic_filters(ctx: &mut PgTokioContext) {
        let client = &ctx.client;
        migrate(client).await;
        assert_eq!(
            queries::count_users(client, count_users_params())
                .await
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
            .await
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
            .await
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
            .await
            .unwrap()
            .total,
            3
        );
        assert!(
            queries::count_users_opt(client, count_users_params())
                .await
                .unwrap()
                .is_some()
        );
    }

    #[test_context(PgTokioContext)]
    #[tokio::test]
    async fn touches_users_with_dynamic_filters(ctx: &mut PgTokioContext) {
        let client = &ctx.client;
        migrate(client).await;
        assert_eq!(
            queries::touch_users(client, Default::default())
                .await
                .unwrap(),
            3
        );
        assert_eq!(
            queries::touch_users(
                client,
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
                client,
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
                client,
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

    #[test_context(PgTokioContext)]
    #[tokio::test]
    async fn reports_dynamic_one_cardinality(ctx: &mut PgTokioContext) {
        let client = &ctx.client;
        migrate(client).await;

        let missing = queries::GetUserByEmailParams {
            email: Some("missing@example.com"),
        };
        assert!(queries::get_user_by_email(client, missing).await.is_err());
        assert!(
            queries::get_user_by_email_opt(
                client,
                queries::GetUserByEmailParams {
                    email: Some("missing@example.com"),
                },
            )
            .await
            .unwrap()
            .is_none()
        );

        let user = queries::get_user_by_email(
            client,
            queries::GetUserByEmailParams {
                email: Some("alice@example.com"),
            },
        )
        .await
        .unwrap();
        assert_eq!(user.id, 1);
        assert_eq!(
            queries::get_user_by_email_opt(
                client,
                queries::GetUserByEmailParams {
                    email: Some("alice@example.com"),
                },
            )
            .await
            .unwrap()
            .unwrap()
            .id,
            1
        );

        assert!(
            queries::get_user_by_email(client, Default::default())
                .await
                .is_err()
        );
        assert!(
            queries::get_user_by_email_opt(client, Default::default())
                .await
                .is_err()
        );
    }

    #[test_context(PgTokioContext)]
    #[tokio::test]
    async fn executes_dynamic_mutations_and_propagates_constraints(ctx: &mut PgTokioContext) {
        let client = &ctx.client;
        migrate(client).await;

        queries::update_user_email(
            client,
            queries::UpdateUserEmailParams {
                new_email: "alice-renamed@example.com",
                id: 1,
                email: Some("alice@example.com"),
            },
        )
        .await
        .unwrap();
        queries::update_user_email(
            client,
            queries::UpdateUserEmailParams {
                new_email: "bob-renamed@example.com",
                id: 2,
                email: None,
            },
        )
        .await
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
            .await
            .is_err()
        );
    }
}

#[cfg(test)]
#[path = "tests/dynamic_stream.rs"]
mod dynamic_stream;

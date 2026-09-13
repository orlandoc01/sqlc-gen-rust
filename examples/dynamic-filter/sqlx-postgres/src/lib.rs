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
        let pool = &ctx.pool;
        migrate(pool).await;

        let users = queries::search_users(pool, params()).await.unwrap();
        assert_eq!(users.len(), 3);
    }

    #[test_context(SqlxPgContext)]
    #[tokio::test]
    async fn applies_each_scalar_filter(ctx: &mut SqlxPgContext) {
        let pool = &ctx.pool;
        migrate(pool).await;

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
        let pool = &ctx.pool;
        migrate(pool).await;

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
        let pool = &ctx.pool;
        migrate(pool).await;

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
    async fn toggles_each_order_by_direction(ctx: &mut SqlxPgContext) {
        let pool = &ctx.pool;
        migrate(pool).await;

        let asc = queries::search_users(
            pool,
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
            pool,
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

    #[test_context(SqlxPgContext)]
    #[tokio::test]
    async fn counts_users_with_dynamic_filters(ctx: &mut SqlxPgContext) {
        let pool = &ctx.pool;
        migrate(pool).await;

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
        let pool = &ctx.pool;
        migrate(pool).await;

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
}

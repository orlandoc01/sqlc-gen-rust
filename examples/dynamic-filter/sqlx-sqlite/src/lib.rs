#[allow(warnings)]
mod queries;

#[cfg(test)]
mod tests {
    use super::queries;
    use std::sync::atomic::{AtomicU64, Ordering};

    async fn pool() -> sqlx::SqlitePool {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "sqlc-gen-rust-dynfilter-{}-{}.db",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        sqlx::SqlitePool::connect(&format!("sqlite://{}?mode=rwc", path.display()))
            .await
            .unwrap()
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

    #[tokio::test]
    async fn searches_without_filters() {
        let pool = pool().await;
        migrate(&pool).await;

        let users = queries::search_users(&pool, params()).await.unwrap();
        assert_eq!(users.len(), 3);
    }

    #[tokio::test]
    async fn applies_each_scalar_filter() {
        let pool = pool().await;
        migrate(&pool).await;

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
        migrate(&pool).await;

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
        migrate(&pool).await;

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
    async fn toggles_each_order_by_direction() {
        let pool = pool().await;
        migrate(&pool).await;

        let asc = queries::search_users(
            &pool,
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
            &pool,
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
}

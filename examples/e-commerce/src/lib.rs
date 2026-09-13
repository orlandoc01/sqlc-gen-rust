#[allow(dead_code)]
mod postgres_query;
#[allow(dead_code)]
mod sqlx_query;
#[allow(dead_code)]
mod tokio_query;

#[cfg(test)]
mod tests {
    use super::*;
    use postgres::fallible_iterator::FallibleIterator as _;
    use test_context::test_context;
    use test_utils::SqlxPgContext;

    #[test_context(SqlxPgContext)]
    #[tokio::test]
    async fn creates_and_lists_users(ctx: &mut SqlxPgContext) {
        sqlx::raw_sql(include_str!("../schema.sql"))
            .execute(&ctx.pool)
            .await
            .unwrap();

        let username = "test_user";
        let email = "test@example.com";
        let user = sqlx_query::create_user(
            &ctx.pool,
            sqlx_query::CreateUserParams {
                username,
                email,
                hashed_password: "password123",
                full_name: Some("Test User"),
            },
        )
        .await
        .unwrap();

        assert_eq!(user.username, username);
        assert_eq!(user.email, email);

        let users = sqlx_query::list_users(
            &ctx.pool,
            sqlx_query::ListUsersParams {
                limit: 100,
                offset: 0,
            },
        )
        .await
        .unwrap();
        assert_eq!(users.len(), 1);
    }

    #[test_context(test_utils::PgTokioContext)]
    #[tokio::test]
    async fn tokio_creates_and_lists_users(ctx: &mut test_utils::PgTokioContext) {
        use futures_util::TryStreamExt as _;

        let client = &ctx.client;
        client
            .batch_execute(include_str!("../schema.sql"))
            .await
            .unwrap();
        let username = "test_user";
        let email = "test@example.com";
        let statement = tokio_query::prepare_create_user(client).await.unwrap();
        let user = tokio_query::create_user_with(
            client,
            &statement,
            tokio_query::CreateUserParams {
                username,
                email,
                hashed_password: "password123",
                full_name: Some("Test User"),
            },
        )
        .await
        .unwrap();
        assert_eq!(user.username, username);
        assert_eq!(user.email, email);

        let users = tokio_query::list_users(
            client,
            tokio_query::ListUsersParams {
                limit: 100,
                offset: 0,
            },
        )
        .await
        .unwrap();
        assert_eq!(users.len(), 1);
        let stream = tokio_query::list_users_stream(
            client,
            tokio_query::ListUsersParams {
                limit: 100,
                offset: 0,
            },
        )
        .await
        .unwrap();
        futures_util::pin_mut!(stream);
        assert_eq!(stream.try_collect::<Vec<_>>().await.unwrap().len(), 1);
    }

    #[test_context(test_utils::PgSyncContext)]
    #[test]
    fn postgres_creates_and_lists_users(ctx: &mut test_utils::PgSyncContext) {
        let client = &mut ctx.client;
        client.batch_execute(include_str!("../schema.sql")).unwrap();
        let username = "test_user";
        let email = "test@example.com";
        let statement = postgres_query::prepare_create_user(client).unwrap();
        let user = postgres_query::create_user_with(
            client,
            &statement,
            postgres_query::CreateUserParams {
                username,
                email,
                hashed_password: "password123",
                full_name: Some("Test User"),
            },
        )
        .unwrap();
        assert_eq!(user.username, username);
        assert_eq!(user.email, email);

        let users = postgres_query::list_users(
            client,
            postgres_query::ListUsersParams {
                limit: 100,
                offset: 0,
            },
        )
        .unwrap();
        assert_eq!(users.len(), 1);
        assert_eq!(
            postgres_query::list_users_iter(
                client,
                postgres_query::ListUsersParams {
                    limit: 100,
                    offset: 0,
                },
            )
            .unwrap()
            .collect::<Vec<_>>()
            .unwrap()
            .len(),
            1
        );
    }
}

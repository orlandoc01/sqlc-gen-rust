#[allow(dead_code)]
mod sqlx_query;

#[cfg(test)]
mod tests {
    use super::*;
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
}

#[allow(dead_code)]
mod queries;

#[cfg(test)]
mod tests {
    use super::*;
    use test_context::test_context;
    use test_utils::SqlxMysqlContext;

    async fn migrate_db(pool: &sqlx::MySqlPool) {
        sqlx::raw_sql(include_str!("../schema.sql"))
            .execute(pool)
            .await
            .unwrap();
    }

    #[test_context(SqlxMysqlContext)]
    #[tokio::test]
    async fn test_authors(ctx: &mut SqlxMysqlContext) {
        let pool = &ctx.pool;
        migrate_db(pool).await;

        let authors = queries::list_authors(pool).await.unwrap();
        assert_eq!(authors.len(), 0);

        let inserted_author = queries::create_author(
            pool,
            queries::CreateAuthorParams {
                name: "Brian Kernighan",
                bio: Some(
                    "Co-author of The C Programming Language and The Go Programming Language",
                ),
            },
        )
        .await
        .unwrap();
        let id: i64 = inserted_author.last_insert_id().try_into().unwrap();

        let fetched_author = queries::get_author(pool, id).await.unwrap();
        assert_eq!(fetched_author.name, "Brian Kernighan");

        assert_eq!(queries::count_authors(pool).await.unwrap().count, 1);

        queries::delete_author(pool, id).await.unwrap();
        assert!(queries::get_author_opt(pool, id).await.unwrap().is_none());
    }
}

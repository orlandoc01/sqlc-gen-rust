#[allow(dead_code)]
mod queries;

#[cfg(test)]
mod tests {
    use super::*;
    use test_context::test_context;
    use test_utils::SqlxSqliteContext;

    async fn migrate_db(pool: &sqlx::SqlitePool) {
        sqlx::raw_sql(include_str!("../schema.sql"))
            .execute(pool)
            .await
            .unwrap();
    }

    #[test_context(SqlxSqliteContext)]
    #[tokio::test]
    async fn test_authors(ctx: &mut SqlxSqliteContext) {
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

        let _fetched_author = queries::get_author(pool, inserted_author.last_insert_rowid())
            .await
            .unwrap();

        assert_eq!(queries::count_authors(pool).await.unwrap().count, 1);

        assert_eq!(
            queries::authors_by_executor(pool, "Brian Kernighan")
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            queries::authors_by_q(
                pool,
                Some("Co-author of The C Programming Language and The Go Programming Language")
            )
            .await
            .unwrap()
            .len(),
            1
        );
    }
}

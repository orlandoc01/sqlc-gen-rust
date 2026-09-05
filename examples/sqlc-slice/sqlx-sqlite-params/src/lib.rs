#[allow(warnings)]
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

    async fn seed_authors(pool: &sqlx::SqlitePool) {
        sqlx::query("INSERT INTO authors (id, name) VALUES (?, ?), (?, ?), (?, ?)")
            .bind(1i64)
            .bind("Alice")
            .bind(2i64)
            .bind("Bob")
            .bind(3i64)
            .bind("Charlie")
            .execute(pool)
            .await
            .unwrap();
    }

    #[test_context(SqlxSqliteContext)]
    #[tokio::test]
    async fn test_list_authors_by_ids(ctx: &mut SqlxSqliteContext) {
        let pool = &ctx.pool;
        migrate_db(pool).await;
        seed_authors(pool).await;

        let authors =
            queries::list_authors_by_i_ds(pool, queries::ListAuthorsByIDsParams { ids: &[] })
                .await
                .unwrap();
        assert_eq!(authors.len(), 0);

        let ids = [2i64];
        let authors =
            queries::list_authors_by_i_ds(pool, queries::ListAuthorsByIDsParams { ids: &ids })
                .await
                .unwrap();
        assert_eq!(authors.len(), 1);
        assert_eq!(authors[0].id, 2);

        let ids = [1i64, 3i64];
        let authors =
            queries::list_authors_by_i_ds(pool, queries::ListAuthorsByIDsParams { ids: &ids })
                .await
                .unwrap();
        assert_eq!(authors.len(), 2);
        assert_eq!(authors[0].id, 1);
        assert_eq!(authors[1].id, 3);
    }

    #[test_context(SqlxSqliteContext)]
    #[tokio::test]
    async fn test_list_authors_by_two_id_lists(ctx: &mut SqlxSqliteContext) {
        let pool = &ctx.pool;
        migrate_db(pool).await;
        seed_authors(pool).await;

        let authors = queries::list_authors_by_two_id_lists(
            pool,
            queries::ListAuthorsByTwoIdListsParams {
                ids: &[],
                backup_ids: &[],
            },
        )
        .await
        .unwrap();
        assert_eq!(authors.len(), 0);

        let ids = [1i64];
        let authors = queries::list_authors_by_two_id_lists(
            pool,
            queries::ListAuthorsByTwoIdListsParams {
                ids: &ids,
                backup_ids: &[],
            },
        )
        .await
        .unwrap();
        assert_eq!(authors.len(), 1);
        assert_eq!(authors[0].id, 1);

        let ids = [1i64, 2i64];
        let backup_ids = [3i64];
        let authors = queries::list_authors_by_two_id_lists(
            pool,
            queries::ListAuthorsByTwoIdListsParams {
                ids: &ids,
                backup_ids: &backup_ids,
            },
        )
        .await
        .unwrap();
        assert_eq!(authors.len(), 3);
    }

    #[test_context(SqlxSqliteContext)]
    #[tokio::test]
    async fn test_list_authors_by_ids_mixed(ctx: &mut SqlxSqliteContext) {
        let pool = &ctx.pool;
        migrate_db(pool).await;
        seed_authors(pool).await;

        let authors = queries::list_authors_by_i_ds_mixed(
            pool,
            queries::ListAuthorsByIDsMixedParams {
                ids: &[],
                id: 1,
                skip_ids: &[],
                name: "X",
            },
        )
        .await
        .unwrap();
        assert_eq!(authors.len(), 0);

        let ids = [1i64];
        let skip_ids = [2i64];
        let authors = queries::list_authors_by_i_ds_mixed(
            pool,
            queries::ListAuthorsByIDsMixedParams {
                ids: &ids,
                id: 1,
                skip_ids: &skip_ids,
                name: "X",
            },
        )
        .await
        .unwrap();
        assert_eq!(authors.len(), 1);
        assert_eq!(authors[0].id, 1);

        let ids = [1i64, 2i64, 3i64];
        let skip_ids = [2i64];
        let authors = queries::list_authors_by_i_ds_mixed(
            pool,
            queries::ListAuthorsByIDsMixedParams {
                ids: &ids,
                id: 1,
                skip_ids: &skip_ids,
                name: "Alice",
            },
        )
        .await
        .unwrap();
        assert_eq!(authors.len(), 1);
        assert_eq!(authors[0].id, 3);
        assert_eq!(authors[0].name, "Charlie");
    }
}

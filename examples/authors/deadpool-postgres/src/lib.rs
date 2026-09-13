#[allow(dead_code)]
mod queries;

#[cfg(test)]
mod tests {
    use futures_util::TryStreamExt as _;
    use test_context::test_context;
    use test_utils::PgDeadpoolContext;

    use super::*;

    async fn migrate_db(client: &deadpool_postgres::Client) {
        client
            .batch_execute(include_str!("../schema.sql"))
            .await
            .unwrap();
    }

    #[test_context(PgDeadpoolContext)]
    #[tokio::test]
    async fn test_authors(ctx: &mut PgDeadpoolContext) {
        let mut client = ctx.pool.get().await.unwrap();
        migrate_db(&client).await;

        assert!(queries::list_authors(&client).await.unwrap().is_empty());
        let stream = queries::list_authors_stream(&client).await.unwrap();
        futures_util::pin_mut!(stream);
        assert!(stream.try_collect::<Vec<_>>().await.unwrap().is_empty());

        let inserted_author = queries::create_author(
            &client,
            queries::CreateAuthorParams {
                name: "Brian Kernighan",
                bio: Some(
                    "Co-author of The C Programming Language and The Go Programming Language",
                ),
            },
        )
        .await
        .unwrap();

        let statement = queries::prepare_get_author(&client).await.unwrap();
        let fetched_author = queries::get_author_with(&client, &statement, inserted_author.id)
            .await
            .unwrap();
        assert_eq!(fetched_author.name, "Brian Kernighan");
        assert_eq!(queries::count_authors(&client).await.unwrap().count, 1);

        let tx = client.transaction().await.unwrap();
        assert_eq!(queries::count_authors(&tx).await.unwrap().count, 1);
        tx.commit().await.unwrap();

        queries::delete_author(&client, inserted_author.id)
            .await
            .unwrap();
        assert!(
            queries::get_author_opt(&client, inserted_author.id)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[test_context(PgDeadpoolContext)]
    #[tokio::test]
    async fn test_keyword_ident(ctx: &mut PgDeadpoolContext) {
        let client = ctx.pool.get().await.unwrap();
        migrate_db(&client).await;
        client
            .execute("INSERT INTO keyword_idents (type) VALUES ('fn')", &[])
            .await
            .unwrap();

        let row = queries::get_keyword_ident(&client, "fn").await.unwrap();
        assert_eq!(row.r#type, "fn");
    }

    #[test_context(PgDeadpoolContext)]
    #[tokio::test]
    async fn reuses_cached_prepared_statements(ctx: &mut PgDeadpoolContext) {
        let client = ctx.pool.get().await.unwrap();
        migrate_db(&client).await;
        let author = queries::create_author(
            &client,
            queries::CreateAuthorParams {
                name: "Cached",
                bio: None,
            },
        )
        .await
        .unwrap();
        let first = queries::prepare_get_author(&client).await.unwrap();
        let second = queries::prepare_get_author(&client).await.unwrap();

        assert_eq!(
            queries::get_author_with(&client, &first, author.id)
                .await
                .unwrap()
                .id,
            author.id
        );
        assert_eq!(
            queries::get_author_with(&client, &second, author.id)
                .await
                .unwrap()
                .id,
            author.id
        );
        assert_eq!(
            client
                .query_one(
                    "SELECT count(*) FROM pg_prepared_statements WHERE statement = $1",
                    &[&queries::GET_AUTHOR],
                )
                .await
                .unwrap()
                .get::<_, i64>(0),
            1
        );
    }

    #[test_context(PgDeadpoolContext)]
    #[tokio::test]
    async fn rolls_back_prepared_writes(ctx: &mut PgDeadpoolContext) {
        let mut client = ctx.pool.get().await.unwrap();
        migrate_db(&client).await;
        let transaction = client.transaction().await.unwrap();
        let statement = queries::prepare_create_author(&transaction).await.unwrap();

        queries::create_author_with(
            &transaction,
            &statement,
            queries::CreateAuthorParams {
                name: "Rolled back",
                bio: None,
            },
        )
        .await
        .unwrap();
        transaction.rollback().await.unwrap();

        assert_eq!(queries::count_authors(&client).await.unwrap().count, 0);
    }
}

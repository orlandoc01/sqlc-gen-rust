#[allow(dead_code)]
mod queries;

#[cfg(test)]
mod tests {
    use futures_util::TryStreamExt as _;
    use test_context::test_context;
    use test_utils::PgTokioContext;

    use super::*;

    async fn migrate_db(client: &tokio_postgres::Client) {
        client
            .batch_execute(include_str!("../schema.sql"))
            .await
            .unwrap();
    }

    async fn seed_authors(client: &tokio_postgres::Client) {
        client
            .execute(
                "INSERT INTO authors (id, name) VALUES ($1, $2), ($3, $4), ($5, $6)",
                &[&1i64, &"Alice", &2i64, &"Bob", &3i64, &"Charlie"],
            )
            .await
            .unwrap();
    }

    #[test_context(PgTokioContext)]
    #[tokio::test]
    async fn lists_authors_by_ids(ctx: &mut PgTokioContext) {
        let client = &ctx.client;
        migrate_db(client).await;
        seed_authors(client).await;

        assert!(
            queries::list_authors_by_ids(client, &[])
                .await
                .unwrap()
                .is_empty()
        );
        let ids = [2i64];
        let statement = queries::prepare_list_authors_by_ids(client).await.unwrap();
        let authors = queries::list_authors_by_ids_with(client, &statement, &ids)
            .await
            .unwrap();
        assert_eq!(authors[0].id, 2);

        let stream = queries::list_authors_by_ids_stream(client, &ids)
            .await
            .unwrap();
        futures_util::pin_mut!(stream);
        assert_eq!(stream.try_collect::<Vec<_>>().await.unwrap().len(), 1);

        let ids = [1i64, 3i64];
        let authors = queries::list_authors_by_ids(client, &ids).await.unwrap();
        assert_eq!(
            authors.iter().map(|author| author.id).collect::<Vec<_>>(),
            [1, 3]
        );
    }

    #[test_context(PgTokioContext)]
    #[tokio::test]
    async fn lists_authors_by_two_id_lists(ctx: &mut PgTokioContext) {
        let client = &ctx.client;
        migrate_db(client).await;
        seed_authors(client).await;

        let authors = queries::list_authors_by_two_id_lists(
            client,
            queries::ListAuthorsByTwoIdListsParams {
                ids: &[],
                backup_ids: &[],
            },
        )
        .await
        .unwrap();
        assert!(authors.is_empty());

        let ids = [1i64];
        let authors = queries::list_authors_by_two_id_lists(
            client,
            queries::ListAuthorsByTwoIdListsParams {
                ids: &ids,
                backup_ids: &[],
            },
        )
        .await
        .unwrap();
        assert_eq!(authors[0].id, 1);

        let ids = [1i64, 2i64];
        let backup_ids = [3i64];
        let authors = queries::list_authors_by_two_id_lists(
            client,
            queries::ListAuthorsByTwoIdListsParams {
                ids: &ids,
                backup_ids: &backup_ids,
            },
        )
        .await
        .unwrap();
        assert_eq!(authors.len(), 3);
    }

    #[test_context(PgTokioContext)]
    #[tokio::test]
    async fn lists_authors_by_ids_mixed(ctx: &mut PgTokioContext) {
        let client = &ctx.client;
        migrate_db(client).await;
        seed_authors(client).await;

        let authors = queries::list_authors_by_ids_mixed(
            client,
            queries::ListAuthorsByIDsMixedParams {
                ids: &[],
                min_id: 1,
                skip_ids: &[],
                excluded_name: "X",
            },
        )
        .await
        .unwrap();
        assert!(authors.is_empty());

        let ids = [1i64];
        let skip_ids = [2i64];
        let authors = queries::list_authors_by_ids_mixed(
            client,
            queries::ListAuthorsByIDsMixedParams {
                ids: &ids,
                min_id: 1,
                skip_ids: &skip_ids,
                excluded_name: "X",
            },
        )
        .await
        .unwrap();
        assert_eq!(authors[0].id, 1);

        let ids = [1i64, 2i64, 3i64];
        let authors = queries::list_authors_by_ids_mixed(
            client,
            queries::ListAuthorsByIDsMixedParams {
                ids: &ids,
                min_id: 1,
                skip_ids: &skip_ids,
                excluded_name: "Alice",
            },
        )
        .await
        .unwrap();
        assert_eq!(authors[0].id, 3);
        assert_eq!(authors[0].name, "Charlie");
    }
}

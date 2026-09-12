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

    #[test_context(PgTokioContext)]
    #[tokio::test]
    async fn test_authors(ctx: &mut PgTokioContext) {
        let client = &ctx.client;
        migrate_db(client).await;

        assert!(queries::list_authors(client).await.unwrap().is_empty());
        let stream = queries::list_authors_stream(client).await.unwrap();
        futures_util::pin_mut!(stream);
        assert!(stream.try_collect::<Vec<_>>().await.unwrap().is_empty());

        let inserted_author = queries::create_author(
            client,
            queries::CreateAuthorParams {
                name: "Brian Kernighan",
                bio: Some(
                    "Co-author of The C Programming Language and The Go Programming Language",
                ),
            },
        )
        .await
        .unwrap();

        let statement = queries::prepare_get_author(client).await.unwrap();
        let fetched_author = queries::get_author_with(client, &statement, inserted_author.id)
            .await
            .unwrap();
        assert_eq!(fetched_author.name, "Brian Kernighan");
        assert_eq!(queries::count_authors(client).await.unwrap().count, 1);

        queries::delete_author(client, inserted_author.id)
            .await
            .unwrap();
        assert!(
            queries::get_author_opt(client, inserted_author.id)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[test_context(PgTokioContext)]
    #[tokio::test]
    async fn test_keyword_ident(ctx: &mut PgTokioContext) {
        let client = &ctx.client;
        migrate_db(client).await;
        client
            .execute("INSERT INTO keyword_idents (type) VALUES ('fn')", &[])
            .await
            .unwrap();

        let row = queries::get_keyword_ident(client, "fn").await.unwrap();
        assert_eq!(row.r#type, "fn");
    }
}

#[allow(dead_code)]
mod queries;

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

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

    #[test_context(PgTokioContext)]
    #[tokio::test]
    async fn rolls_back_prepared_writes(ctx: &mut PgTokioContext) {
        migrate_db(&ctx.client).await;
        let transaction = ctx.client.transaction().await.unwrap();
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

        assert_eq!(queries::count_authors(&ctx.client).await.unwrap().count, 0);
    }

    #[test_context(PgTokioContext)]
    #[tokio::test]
    async fn reports_required_optional_decode_and_constraint_errors(ctx: &mut PgTokioContext) {
        let client = &ctx.client;
        migrate_db(client).await;

        assert!(
            queries::get_authors_by_name(client, "missing")
                .await
                .is_err()
        );
        assert!(
            queries::get_authors_by_name_opt(client, "missing")
                .await
                .unwrap()
                .is_none()
        );

        for (id, name) in [(1, "Ada"), (2, "duplicate"), (3, "duplicate")] {
            assert_eq!(
                queries::create_author_with_id(
                    client,
                    queries::CreateAuthorWithIdParams { id, name },
                )
                .await
                .unwrap(),
                1
            );
        }
        assert!(
            queries::create_author_with_id(
                client,
                queries::CreateAuthorWithIdParams {
                    id: 1,
                    name: "Grace",
                },
            )
            .await
            .is_err()
        );
        assert!(
            queries::get_authors_by_name(client, "duplicate")
                .await
                .is_err()
        );
        assert!(
            queries::get_authors_by_name_opt(client, "duplicate")
                .await
                .is_err()
        );

        let mismatched = "SELECT id, 42, bio FROM authors WHERE id = $1 LIMIT 1";
        assert!(
            queries::get_author_with(client, mismatched, 1)
                .await
                .is_err()
        );
        assert!(
            queries::get_author_opt_with(client, mismatched, 1)
                .await
                .is_err()
        );
    }

    #[test_context(PgTokioContext)]
    #[tokio::test]
    async fn reports_execution_counts_and_executes_returning_queries(ctx: &mut PgTokioContext) {
        let client = &ctx.client;
        migrate_db(client).await;

        assert_eq!(queries::touch_authors(client, 99).await.unwrap(), 0);
        for (id, name) in [(1, "Ada"), (2, "Grace")] {
            queries::create_author_with_id(client, queries::CreateAuthorWithIdParams { id, name })
                .await
                .unwrap();
        }
        assert_eq!(queries::touch_authors(client, 1).await.unwrap(), 2);
        assert_eq!(
            queries::rename_authors_returning_id(
                client,
                queries::RenameAuthorsReturningIdParams {
                    name: "Renamed",
                    id: 99,
                },
            )
            .await
            .unwrap(),
            0
        );
        assert_eq!(
            queries::rename_authors_returning_id(
                client,
                queries::RenameAuthorsReturningIdParams {
                    name: "Renamed",
                    id: 1,
                },
            )
            .await
            .unwrap(),
            2
        );
        queries::delete_author_returning_id(client, 1)
            .await
            .unwrap();
        assert_eq!(queries::count_authors(client).await.unwrap().count, 1);
        queries::delete_author_returning_id(client, 1)
            .await
            .unwrap();
        assert_eq!(queries::count_authors(client).await.unwrap().count, 1);
    }

    #[test_context(PgTokioContext)]
    #[tokio::test]
    async fn round_trips_default_system_time_mappings(ctx: &mut PgTokioContext) {
        let client = &ctx.client;
        migrate_db(client).await;
        let timestamp = UNIX_EPOCH - Duration::from_micros(1);
        let timestamptz = UNIX_EPOCH + Duration::new(1_700_000_000, 123_456_000);

        queries::insert_timestamps(
            client,
            queries::InsertTimestampsParams {
                id: 1,
                timestamp_val: timestamp,
                timestamptz_val: timestamptz,
                nullable_timestamp: None,
            },
        )
        .await
        .unwrap();
        let row = queries::get_timestamps(client, 1).await.unwrap();
        assert_eq!(row.timestamp_val, timestamp);
        assert_eq!(row.timestamptz_val, timestamptz);
        assert_eq!(row.nullable_timestamp, None);
    }
}

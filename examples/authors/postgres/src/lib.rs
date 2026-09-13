#[allow(dead_code)]
mod locals;
#[allow(dead_code)]
mod queries;

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use postgres::fallible_iterator::FallibleIterator as _;
    use test_context::test_context;
    use test_utils::PgSyncContext;

    use super::*;

    fn migrate_db(client: &mut postgres::Client) {
        client.batch_execute(include_str!("../schema.sql")).unwrap();
    }

    #[test_context(PgSyncContext)]
    #[test]
    fn test_authors(ctx: &mut PgSyncContext) {
        let client = &mut ctx.client;
        migrate_db(client);

        assert!(queries::list_authors(client).unwrap().is_empty());
        assert!(
            queries::list_authors_iter(client)
                .unwrap()
                .collect::<Vec<_>>()
                .unwrap()
                .is_empty()
        );

        let inserted_author = queries::create_author(
            client,
            queries::CreateAuthorParams {
                name: "Brian Kernighan",
                bio: Some(
                    "Co-author of The C Programming Language and The Go Programming Language",
                ),
            },
        )
        .unwrap();

        let statement = queries::prepare_get_author(client).unwrap();
        let fetched_author =
            queries::get_author_with(client, &statement, inserted_author.id).unwrap();
        assert_eq!(fetched_author.name, "Brian Kernighan");
        assert_eq!(queries::count_authors(client).unwrap().count, 1);

        queries::delete_author(client, inserted_author.id).unwrap();
        assert!(
            queries::get_author_opt(client, inserted_author.id)
                .unwrap()
                .is_none()
        );
    }

    #[test_context(PgSyncContext)]
    #[test]
    fn test_keyword_ident(ctx: &mut PgSyncContext) {
        let client = &mut ctx.client;
        migrate_db(client);
        client
            .execute("INSERT INTO keyword_idents (type) VALUES ('fn')", &[])
            .unwrap();

        let row = queries::get_keyword_ident(client, "fn").unwrap();
        assert_eq!(row.r#type, "fn");
    }

    #[test_context(PgSyncContext)]
    #[test]
    fn rolls_back_prepared_writes(ctx: &mut PgSyncContext) {
        migrate_db(&mut ctx.client);
        let mut transaction = ctx.client.transaction().unwrap();
        let statement = queries::prepare_create_author(&mut transaction).unwrap();

        queries::create_author_with(
            &mut transaction,
            &statement,
            queries::CreateAuthorParams {
                name: "Rolled back",
                bio: None,
            },
        )
        .unwrap();
        transaction.rollback().unwrap();

        assert_eq!(queries::count_authors(&mut ctx.client).unwrap().count, 0);
    }

    #[test_context(PgSyncContext)]
    #[test]
    fn reports_required_optional_decode_and_constraint_errors(ctx: &mut PgSyncContext) {
        let client = &mut ctx.client;
        migrate_db(client);

        assert!(queries::get_authors_by_name(client, "missing").is_err());
        assert!(
            queries::get_authors_by_name_opt(client, "missing")
                .unwrap()
                .is_none()
        );

        for (id, name) in [(1, "Ada"), (2, "duplicate"), (3, "duplicate")] {
            assert_eq!(
                queries::create_author_with_id(
                    client,
                    queries::CreateAuthorWithIdParams { id, name },
                )
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
            .is_err()
        );
        assert!(queries::get_authors_by_name(client, "duplicate").is_err());
        assert!(queries::get_authors_by_name_opt(client, "duplicate").is_err());

        let mismatched = "SELECT id, 42, bio FROM authors WHERE id = $1 LIMIT 1";
        assert!(queries::get_author_with(client, mismatched, 1).is_err());
        assert!(queries::get_author_opt_with(client, mismatched, 1).is_err());
    }

    #[test_context(PgSyncContext)]
    #[test]
    fn reports_execution_counts_and_executes_returning_queries(ctx: &mut PgSyncContext) {
        let client = &mut ctx.client;
        migrate_db(client);

        assert_eq!(queries::touch_authors(client, 99).unwrap(), 0);
        for (id, name) in [(1, "Ada"), (2, "Grace")] {
            queries::create_author_with_id(client, queries::CreateAuthorWithIdParams { id, name })
                .unwrap();
        }
        assert_eq!(queries::touch_authors(client, 1).unwrap(), 2);
        assert_eq!(
            queries::rename_authors_returning_id(
                client,
                queries::RenameAuthorsReturningIdParams {
                    name: "Renamed",
                    id: 99,
                },
            )
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
            .unwrap(),
            2
        );
        queries::delete_author_returning_id(client, 1).unwrap();
        assert_eq!(queries::count_authors(client).unwrap().count, 1);
        queries::delete_author_returning_id(client, 1).unwrap();
        assert_eq!(queries::count_authors(client).unwrap().count, 1);
    }

    #[test_context(PgSyncContext)]
    #[test]
    fn round_trips_default_system_time_mappings(ctx: &mut PgSyncContext) {
        let client = &mut ctx.client;
        migrate_db(client);
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
        .unwrap();
        let row = queries::get_timestamps(client, 1).unwrap();
        assert_eq!(row.timestamp_val, timestamp);
        assert_eq!(row.timestamptz_val, timestamptz);
        assert_eq!(row.nullable_timestamp, None);
    }

    #[test_context(PgSyncContext)]
    #[test]
    fn binds_direct_parameters_named_like_generator_locals(ctx: &mut PgSyncContext) {
        let client = &mut ctx.client;
        migrate_db(client);
        for (id, name) in [(1, "Client"), (2, "Statement"), (3, "Values")] {
            queries::create_author_with_id(client, queries::CreateAuthorWithIdParams { id, name })
                .unwrap();
        }

        assert_eq!(
            locals::list_authors_by_local_names(client, "Client", "missing", "missing")
                .unwrap()
                .iter()
                .map(|author| author.id)
                .collect::<Vec<_>>(),
            [1]
        );
        let statement = locals::prepare_list_authors_by_local_names(client).unwrap();
        assert_eq!(
            locals::list_authors_by_local_names_with(
                client,
                &statement,
                "missing",
                "Statement",
                "missing",
            )
            .unwrap()
            .iter()
            .map(|author| author.id)
            .collect::<Vec<_>>(),
            [2]
        );
    }
}

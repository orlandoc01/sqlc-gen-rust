#[allow(dead_code)]
mod queries;

#[cfg(test)]
mod tests {
    use postgres::fallible_iterator::FallibleIterator as _;
    use test_context::test_context;
    use test_utils::PgSyncContext;

    use super::*;

    fn migrate_db(client: &mut postgres::Client) {
        client.batch_execute(include_str!("../schema.sql")).unwrap();
    }

    fn seed_authors(client: &mut postgres::Client) {
        client
            .execute(
                "INSERT INTO authors (id, name) VALUES ($1, $2), ($3, $4), ($5, $6)",
                &[&1i64, &"Alice", &2i64, &"Bob", &3i64, &"Charlie"],
            )
            .unwrap();
    }

    #[test_context(PgSyncContext)]
    #[test]
    fn lists_authors_by_ids(ctx: &mut PgSyncContext) {
        let client = &mut ctx.client;
        migrate_db(client);
        seed_authors(client);

        assert!(
            queries::list_authors_by_ids(client, &[])
                .unwrap()
                .is_empty()
        );
        let ids = [2i64];
        let statement = queries::prepare_list_authors_by_ids(client).unwrap();
        let authors = queries::list_authors_by_ids_with(client, &statement, &ids).unwrap();
        assert_eq!(authors[0].id, 2);

        assert_eq!(
            queries::list_authors_by_ids_iter(client, &ids)
                .unwrap()
                .collect::<Vec<_>>()
                .unwrap()
                .len(),
            1
        );

        let ids = [1i64, 3i64];
        let authors = queries::list_authors_by_ids(client, &ids).unwrap();
        assert_eq!(
            authors.iter().map(|author| author.id).collect::<Vec<_>>(),
            [1, 3]
        );
    }

    #[test_context(PgSyncContext)]
    #[test]
    fn lists_authors_by_two_id_lists(ctx: &mut PgSyncContext) {
        let client = &mut ctx.client;
        migrate_db(client);
        seed_authors(client);

        let authors = queries::list_authors_by_two_id_lists(
            client,
            queries::ListAuthorsByTwoIdListsParams {
                ids: &[],
                backup_ids: &[],
            },
        )
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
        .unwrap();
        assert_eq!(authors.len(), 3);
    }

    #[test_context(PgSyncContext)]
    #[test]
    fn lists_authors_by_ids_mixed(ctx: &mut PgSyncContext) {
        let client = &mut ctx.client;
        migrate_db(client);
        seed_authors(client);

        let authors = queries::list_authors_by_ids_mixed(
            client,
            queries::ListAuthorsByIDsMixedParams {
                ids: &[],
                min_id: 1,
                skip_ids: &[],
                excluded_name: "X",
            },
        )
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
        .unwrap();
        assert_eq!(authors[0].id, 3);
        assert_eq!(authors[0].name, "Charlie");
    }
}

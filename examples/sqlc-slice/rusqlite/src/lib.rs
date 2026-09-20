#[allow(dead_code)]
mod queries;

#[cfg(test)]
mod tests {
    use super::*;
    use test_context::test_context;
    use test_utils::RusqliteContext;

    fn migrate_db(conn: &rusqlite::Connection) {
        conn.execute_batch(include_str!("../../sqlx-sqlite/schema.sql"))
            .unwrap();
    }

    fn seed_authors(conn: &rusqlite::Connection) {
        conn.execute(
            "INSERT INTO authors (id, name) VALUES (?, ?), (?, ?), (?, ?)",
            rusqlite::params![1i64, "Alice", 2i64, "Bob", 3i64, "Charlie"],
        )
        .unwrap();
    }

    #[test_context(RusqliteContext)]
    #[test]
    fn test_list_authors_by_ids(ctx: &mut RusqliteContext) {
        let conn = &ctx.conn;
        migrate_db(conn);
        seed_authors(conn);

        assert!(
            queries::list_authors_by_ids(conn, queries::ListAuthorsByIDsParams { ids: &[] })
                .unwrap()
                .is_empty()
        );
        let ids = [2i64];
        let authors =
            queries::list_authors_by_ids(conn, queries::ListAuthorsByIDsParams { ids: &ids })
                .unwrap();
        assert_eq!(authors.len(), 1);
        assert_eq!(authors[0].id, 2);

        let ids = [1i64, 3i64];
        let authors =
            queries::list_authors_by_ids(conn, queries::ListAuthorsByIDsParams { ids: &ids })
                .unwrap();
        assert_eq!(authors.len(), 2);
        assert_eq!(authors[0].id, 1);
        assert_eq!(authors[1].id, 3);
    }

    #[test_context(RusqliteContext)]
    #[test]
    fn keeps_slice_marker_text_inside_literals(ctx: &mut RusqliteContext) {
        let conn = &ctx.conn;
        migrate_db(conn);
        seed_authors(conn);

        let ids = [1i64, 3i64];
        let rows = queries::list_author_labels_by_ids(
            conn,
            queries::ListAuthorLabelsByIDsParams { ids: &ids },
        )
        .unwrap();
        assert_eq!(
            rows.iter()
                .map(|row| (row.id, row.label.as_str()))
                .collect::<Vec<_>>(),
            [(1, "/*SLICE:ids*/?"), (3, "/*SLICE:ids*/?")]
        );
    }

    #[test_context(RusqliteContext)]
    #[test]
    fn test_list_authors_by_two_id_lists(ctx: &mut RusqliteContext) {
        let conn = &ctx.conn;
        migrate_db(conn);
        seed_authors(conn);

        let authors = queries::list_authors_by_two_id_lists(
            conn,
            queries::ListAuthorsByTwoIdListsParams {
                ids: &[],
                backup_ids: &[],
            },
        )
        .unwrap();
        assert!(authors.is_empty());

        let ids = [1i64];
        let authors = queries::list_authors_by_two_id_lists(
            conn,
            queries::ListAuthorsByTwoIdListsParams {
                ids: &ids,
                backup_ids: &[],
            },
        )
        .unwrap();
        assert_eq!(authors.len(), 1);
        assert_eq!(authors[0].id, 1);

        let ids = [1i64, 2i64];
        let backup_ids = [3i64];
        let authors = queries::list_authors_by_two_id_lists(
            conn,
            queries::ListAuthorsByTwoIdListsParams {
                ids: &ids,
                backup_ids: &backup_ids,
            },
        )
        .unwrap();
        assert_eq!(authors.len(), 3);
    }

    #[test_context(RusqliteContext)]
    #[test]
    fn test_list_authors_by_ids_mixed(ctx: &mut RusqliteContext) {
        let conn = &ctx.conn;
        migrate_db(conn);
        seed_authors(conn);

        let authors = queries::list_authors_by_ids_mixed(
            conn,
            queries::ListAuthorsByIDsMixedParams {
                ids: &[],
                id: 1,
                skip_ids: &[],
                name: "X",
            },
        )
        .unwrap();
        assert!(authors.is_empty());

        let ids = [1i64];
        let skip_ids = [2i64];
        let authors = queries::list_authors_by_ids_mixed(
            conn,
            queries::ListAuthorsByIDsMixedParams {
                ids: &ids,
                id: 1,
                skip_ids: &skip_ids,
                name: "X",
            },
        )
        .unwrap();
        assert_eq!(authors.len(), 1);
        assert_eq!(authors[0].id, 1);

        let ids = [1i64, 2i64, 3i64];
        let skip_ids = [2i64];
        let authors = queries::list_authors_by_ids_mixed(
            conn,
            queries::ListAuthorsByIDsMixedParams {
                ids: &ids,
                id: 1,
                skip_ids: &skip_ids,
                name: "Alice",
            },
        )
        .unwrap();
        assert_eq!(authors.len(), 1);
        assert_eq!(authors[0].id, 3);
        assert_eq!(authors[0].name, "Charlie");
    }

    #[test_context(RusqliteContext)]
    #[test]
    fn test_list_authors_by_named_ids(ctx: &mut RusqliteContext) {
        let conn = &ctx.conn;
        migrate_db(conn);
        seed_authors(conn);

        assert!(
            queries::list_authors_by_named_ids(
                conn,
                queries::ListAuthorsByNamedIDsParams {
                    min_id: 1,
                    ids: &[],
                    max_id: 3,
                },
            )
            .unwrap()
            .is_empty()
        );

        let ids = [2i64];
        let authors = queries::list_authors_by_named_ids(
            conn,
            queries::ListAuthorsByNamedIDsParams {
                min_id: 1,
                ids: &ids,
                max_id: 3,
            },
        )
        .unwrap();
        assert_eq!(
            authors.iter().map(|author| author.id).collect::<Vec<_>>(),
            [2]
        );

        let ids = [1i64, 2, 3];
        let authors = queries::list_authors_by_named_ids(
            conn,
            queries::ListAuthorsByNamedIDsParams {
                min_id: 1,
                ids: &ids,
                max_id: 3,
            },
        )
        .unwrap();
        assert_eq!(
            authors.iter().map(|author| author.id).collect::<Vec<_>>(),
            [1, 2, 3]
        );
    }

    #[test_context(RusqliteContext)]
    #[test]
    fn test_delete_authors_by_ids(ctx: &mut RusqliteContext) {
        let conn = &ctx.conn;
        migrate_db(conn);
        seed_authors(conn);
        let remaining = |conn| {
            queries::list_authors_by_ids(conn, queries::ListAuthorsByIDsParams { ids: &[1, 2, 3] })
                .unwrap()
                .iter()
                .map(|author| author.id)
                .collect::<Vec<_>>()
        };

        queries::delete_authors_by_ids(conn, queries::DeleteAuthorsByIDsParams { ids: &[] })
            .unwrap();
        assert_eq!(remaining(conn), [1, 2, 3]);

        let ids = [1i64, 3];
        queries::delete_authors_by_ids(conn, queries::DeleteAuthorsByIDsParams { ids: &ids })
            .unwrap();
        assert_eq!(remaining(conn), [2]);
    }
}

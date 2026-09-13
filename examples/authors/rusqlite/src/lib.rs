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

    #[test_context(RusqliteContext)]
    #[test]
    fn test_authors(ctx: &mut RusqliteContext) {
        let conn = &mut ctx.conn;
        migrate_db(conn);

        let mut transaction = conn.transaction().unwrap();
        assert!(queries::list_authors(&transaction).unwrap().is_empty());
        let id = queries::create_author(
            &transaction,
            queries::CreateAuthorParams {
                name: "Brian Kernighan",
                bio: Some(
                    "Co-author of The C Programming Language and The Go Programming Language",
                ),
            },
        )
        .unwrap();
        let savepoint = transaction.savepoint().unwrap();
        queries::get_author(&savepoint, id).unwrap();
        savepoint.commit().unwrap();
        transaction.commit().unwrap();

        assert_eq!(queries::count_authors(conn).unwrap().count, 1);
        assert!(queries::get_author_opt(conn, id + 1).unwrap().is_none());
        queries::delete_author(conn, id).unwrap();
        assert_eq!(queries::count_authors(conn).unwrap().count, 0);
    }

    #[test_context(RusqliteContext)]
    #[test]
    fn missing_rows_and_decoding_errors(ctx: &mut RusqliteContext) {
        let conn = &ctx.conn;
        migrate_db(conn);

        assert!(matches!(
            queries::get_author(conn, 404),
            Err(rusqlite::Error::QueryReturnedNoRows)
        ));
        assert!(queries::get_author_opt(conn, 404).unwrap().is_none());

        // A BLOB in a TEXT column is stored as-is, so decoding `name` as String must fail.
        conn.execute("INSERT INTO authors (id, name) VALUES (7, X'00FF')", [])
            .unwrap();
        assert!(matches!(
            queries::get_author_opt(conn, 7),
            Err(rusqlite::Error::InvalidColumnType(
                1,
                _,
                rusqlite::types::Type::Blob
            ))
        ));
        assert!(queries::get_author(conn, 7).is_err());
    }

    #[test_context(RusqliteContext)]
    #[test]
    fn constraint_violations_do_not_report_stale_ids(ctx: &mut RusqliteContext) {
        let conn = &ctx.conn;
        migrate_db(conn);

        assert_eq!(
            queries::create_author_with_id(
                conn,
                queries::CreateAuthorWithIdParams { id: 1, name: "Ada" },
            )
            .unwrap(),
            1
        );
        assert!(matches!(
            queries::create_author_with_id(
                conn,
                queries::CreateAuthorWithIdParams {
                    id: 1,
                    name: "Grace",
                },
            ),
            Err(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error {
                    code: rusqlite::ErrorCode::ConstraintViolation,
                    ..
                },
                _
            ))
        ));
        assert_eq!(queries::count_authors(conn).unwrap().count, 1);
        assert_eq!(queries::get_author(conn, 1).unwrap().name, "Ada");
    }

    #[test_context(RusqliteContext)]
    #[test]
    fn returning_clauses_complete_the_write(ctx: &mut RusqliteContext) {
        let conn = &ctx.conn;
        migrate_db(conn);

        let id = queries::create_author_returning_id(
            conn,
            queries::CreateAuthorReturningIdParams {
                name: "Ada",
                bio: None,
            },
        )
        .unwrap();
        assert_eq!(queries::get_author(conn, id).unwrap().name, "Ada");

        assert_eq!(
            queries::rename_author_returning_id(
                conn,
                queries::RenameAuthorReturningIdParams { name: "Grace", id },
            )
            .unwrap(),
            1
        );
        assert_eq!(
            queries::rename_author_returning_id(
                conn,
                queries::RenameAuthorReturningIdParams {
                    name: "Nobody",
                    id: id + 1,
                },
            )
            .unwrap(),
            0
        );
        assert_eq!(queries::get_author(conn, id).unwrap().name, "Grace");

        queries::delete_author_returning_id(conn, id).unwrap();
        queries::delete_author_returning_id(conn, id).unwrap();
        assert_eq!(queries::count_authors(conn).unwrap().count, 0);
    }

    #[test_context(RusqliteContext)]
    #[test]
    fn rolls_back_transactions_and_savepoints(ctx: &mut RusqliteContext) {
        let conn = &mut ctx.conn;
        migrate_db(conn);

        let transaction = conn.transaction().unwrap();
        queries::create_author(
            &transaction,
            queries::CreateAuthorParams {
                name: "Rolled back",
                bio: None,
            },
        )
        .unwrap();
        transaction.rollback().unwrap();
        assert_eq!(queries::count_authors(conn).unwrap().count, 0);

        let mut transaction = conn.transaction().unwrap();
        let kept = queries::create_author(
            &transaction,
            queries::CreateAuthorParams {
                name: "Kept",
                bio: None,
            },
        )
        .unwrap();
        let mut savepoint = transaction.savepoint().unwrap();
        queries::create_author(
            &savepoint,
            queries::CreateAuthorParams {
                name: "Dropped",
                bio: None,
            },
        )
        .unwrap();
        savepoint.rollback().unwrap();
        drop(savepoint);
        transaction.commit().unwrap();

        assert_eq!(queries::count_authors(conn).unwrap().count, 1);
        assert_eq!(queries::get_author(conn, kept).unwrap().name, "Kept");
    }

    #[test_context(RusqliteContext)]
    #[test]
    fn direct_parameters_named_like_generated_locals(ctx: &mut RusqliteContext) {
        let conn = &ctx.conn;
        migrate_db(conn);
        queries::create_author(
            conn,
            queries::CreateAuthorParams {
                name: "client",
                bio: Some("statement"),
            },
        )
        .unwrap();

        assert_eq!(queries::authors_by_client(conn, "client").unwrap().len(), 1);
        assert!(
            queries::authors_by_client(conn, "statement")
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            queries::authors_by_statement(conn, Some("statement"))
                .unwrap()
                .len(),
            1
        );
        assert_eq!(queries::authors_by_params(conn, "client").unwrap().len(), 1);
    }
}

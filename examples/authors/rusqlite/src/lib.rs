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

    fn count(conn: &rusqlite::Connection) -> i64 {
        queries::count_authors(conn).unwrap().count
    }

    fn author<'a>(name: &'a str, bio: Option<&'a str>) -> queries::CreateAuthorParams<'a> {
        queries::CreateAuthorParams { name, bio }
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
            author(
                "Brian Kernighan",
                Some("Co-author of The C Programming Language and The Go Programming Language"),
            ),
        )
        .unwrap();
        let savepoint = transaction.savepoint().unwrap();
        queries::get_author(&savepoint, id).unwrap();
        savepoint.commit().unwrap();
        transaction.commit().unwrap();

        assert_eq!(count(conn), 1);
        assert!(queries::get_author_opt(conn, id + 1).unwrap().is_none());
        queries::delete_author(conn, id).unwrap();
        assert_eq!(count(conn), 0);
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

        let params = |name| queries::CreateAuthorWithIdParams { id: 1, name };
        assert_eq!(
            queries::create_author_with_id(conn, params("Ada")).unwrap(),
            1
        );
        assert!(matches!(
            queries::create_author_with_id(conn, params("Grace")),
            Err(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error {
                    code: rusqlite::ErrorCode::ConstraintViolation,
                    ..
                },
                _
            ))
        ));
        assert_eq!(count(conn), 1);
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

        let rename = |name, id| queries::RenameAuthorReturningIdParams { name, id };
        assert_eq!(
            queries::rename_author_returning_id(conn, rename("Grace", id)).unwrap(),
            1
        );
        assert_eq!(
            queries::rename_author_returning_id(conn, rename("Nobody", id + 1)).unwrap(),
            0
        );
        assert_eq!(queries::get_author(conn, id).unwrap().name, "Grace");

        queries::delete_author_returning_id(conn, id).unwrap();
        queries::delete_author_returning_id(conn, id).unwrap();
        assert_eq!(count(conn), 0);
    }

    #[test_context(RusqliteContext)]
    #[test]
    fn rolls_back_transactions_and_savepoints(ctx: &mut RusqliteContext) {
        let conn = &mut ctx.conn;
        migrate_db(conn);

        let transaction = conn.transaction().unwrap();
        queries::create_author(&transaction, author("Rolled back", None)).unwrap();
        transaction.rollback().unwrap();
        assert_eq!(count(conn), 0);

        let mut transaction = conn.transaction().unwrap();
        let kept = queries::create_author(&transaction, author("Kept", None)).unwrap();
        let mut savepoint = transaction.savepoint().unwrap();
        queries::create_author(&savepoint, author("Dropped", None)).unwrap();
        savepoint.rollback().unwrap();
        drop(savepoint);
        transaction.commit().unwrap();

        assert_eq!(count(conn), 1);
        assert_eq!(queries::get_author(conn, kept).unwrap().name, "Kept");
    }

    #[test_context(RusqliteContext)]
    #[test]
    fn direct_parameters_named_like_generated_locals(ctx: &mut RusqliteContext) {
        let conn = &ctx.conn;
        migrate_db(conn);
        queries::create_author(conn, author("client", Some("statement"))).unwrap();

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

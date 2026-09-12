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
}

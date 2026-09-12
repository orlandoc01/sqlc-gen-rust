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

    fn seed_reviews(conn: &rusqlite::Connection) {
        conn.execute_batch(
            "INSERT INTO authors (id, name) VALUES (101, 'Ada'), (102, 'Grace');
             INSERT INTO books (id, author_id, title, subtitle) VALUES (201, 101, 'Algorithms', NULL), (202, 102, 'Compilers', 'Second Edition');
             INSERT INTO reviews (id, book_id, rating) VALUES (301, 201, 5), (302, 202, 3);",
        )
        .unwrap();
    }

    #[test_context(RusqliteContext)]
    #[test]
    fn decodes_embedded_rows_by_position(ctx: &mut RusqliteContext) {
        let conn = &ctx.conn;
        migrate_db(conn);
        seed_reviews(conn);

        let review =
            queries::get_review(conn, queries::GetReviewParams { review_id: 301 }).unwrap();
        assert_eq!(review.review_id, 301);
        assert_eq!(review.authors.id, 101);
        assert_eq!(review.books.id, 201);
        assert_eq!(review.books.title, "Algorithms");
        assert!(review.books.subtitle.is_none());
        assert_eq!(review.rating, 5);

        let reviews = queries::list_reviews(conn).unwrap();
        assert_eq!(reviews.len(), 2);
        assert_eq!(reviews[1].review_id, 302);
        assert_eq!(reviews[1].authors.id, 102);
        assert_eq!(reviews[1].books.id, 202);
        assert_eq!(reviews[1].books.subtitle.as_deref(), Some("Second Edition"));
        assert_eq!(reviews[1].rating, 3);

        let reviews = queries::list_reviews_by_minimum_rating(
            conn,
            queries::ListReviewsByMinimumRatingParams {
                min_rating: Some(4),
            },
        )
        .unwrap();
        assert_eq!(reviews.len(), 1);
        assert_eq!(reviews[0].books.id, 201);
    }
}

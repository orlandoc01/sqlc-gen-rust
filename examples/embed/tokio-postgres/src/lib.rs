#[allow(warnings)]
mod queries;

#[cfg(test)]
mod tests {
    use super::*;
    use test_context::test_context;
    use test_utils::PgTokioTestContext;

    #[test_context(PgTokioTestContext)]
    #[tokio::test]
    async fn decodes_embedded_rows_by_position(ctx: &mut PgTokioTestContext) {
        let client = &ctx.client;
        client
            .batch_execute(include_str!("../schema.sql"))
            .await
            .unwrap();
        client
            .batch_execute(
                "INSERT INTO authors (id, name) VALUES (101, 'Ada'), (102, 'Grace');
                 INSERT INTO books (id, author_id, title) VALUES (201, 101, 'Algorithms'), (202, 102, 'Compilers');
                 INSERT INTO reviews (id, book_id, rating) VALUES (301, 201, 5), (302, 202, 3);",
            )
            .await
            .unwrap();

        let review = queries::GetReview::builder()
            .review_id(301)
            .build()
            .query_one(client)
            .await
            .unwrap();
        assert_eq!(review.review_id, 301);
        assert_eq!(review.authors.id, 101);
        assert_eq!(review.books.id, 201);
        assert_eq!(review.rating, 5);
    }
}

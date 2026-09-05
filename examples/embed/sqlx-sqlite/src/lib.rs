#[allow(warnings)]
mod queries;

#[cfg(test)]
mod tests {
    use super::*;
    use test_context::test_context;
    use test_utils::SqlxSqliteContext;

    async fn migrate_db(pool: &sqlx::SqlitePool) {
        sqlx::raw_sql(include_str!("../schema.sql"))
            .execute(pool)
            .await
            .unwrap();
    }

    async fn seed_reviews(pool: &sqlx::SqlitePool) {
        sqlx::raw_sql(
            "INSERT INTO authors (id, name) VALUES (101, 'Ada'), (102, 'Grace');
             INSERT INTO books (id, author_id, title) VALUES (201, 101, 'Algorithms'), (202, 102, 'Compilers');
             INSERT INTO reviews (id, book_id, rating) VALUES (301, 201, 5), (302, 202, 3);",
        )
        .execute(pool)
        .await
        .unwrap();
    }

    #[test_context(SqlxSqliteContext)]
    #[tokio::test]
    async fn decodes_embedded_rows_by_position(ctx: &mut SqlxSqliteContext) {
        let pool = &ctx.pool;
        migrate_db(pool).await;
        seed_reviews(pool).await;

        let review = queries::GetReview::builder()
            .review_id(301)
            .build()
            .query_one(pool)
            .await
            .unwrap();
        assert_eq!(review.review_id, 301);
        assert_eq!(review.authors.id, 101);
        assert_eq!(review.books.id, 201);
        assert_eq!(review.rating, 5);

        let reviews = queries::ListReviews.query_many(pool).await.unwrap();
        assert_eq!(reviews.len(), 2);
        assert_eq!(reviews[1].review_id, 302);
        assert_eq!(reviews[1].authors.id, 102);
        assert_eq!(reviews[1].books.id, 202);

        let reviews = queries::ListReviewsByMinimumRating::builder()
            .min_rating(Some(4))
            .build()
            .query_many(pool)
            .await
            .unwrap();
        assert_eq!(reviews.len(), 1);
        assert_eq!(reviews[0].books.id, 201);
    }
}

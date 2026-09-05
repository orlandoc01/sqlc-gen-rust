/* name: GetReview :one */
SELECT r.id AS review_id, sqlc.embed(a), sqlc.embed(b), r.rating
FROM reviews r
JOIN books b ON b.id = r.book_id
JOIN authors a ON a.id = b.author_id
WHERE r.id = sqlc.arg(review_id);

/* name: ListReviews :many */
SELECT r.id AS review_id, sqlc.embed(a), sqlc.embed(b), r.rating
FROM reviews r
JOIN books b ON b.id = r.book_id
JOIN authors a ON a.id = b.author_id
ORDER BY r.id;

/* name: ListReviewsByMinimumRating :many */
SELECT r.id AS review_id, sqlc.embed(a), sqlc.embed(b), r.rating
FROM reviews r
JOIN books b ON b.id = r.book_id
JOIN authors a ON a.id = b.author_id
WHERE r.rating >= sqlc.narg('min_rating')
ORDER BY r.id;

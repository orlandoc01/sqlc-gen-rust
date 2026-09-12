/* name: GetAuthor :one */
SELECT * FROM authors
WHERE id = ? LIMIT 1;

/* name: ListAuthors :many */
SELECT * FROM authors
ORDER BY name;

-- name: CountAuthors :one
SELECT COUNT(*) FROM authors;

/* name: CreateAuthor :execresult */
INSERT INTO authors (
  name, bio
) VALUES (
  ?, ?
);

/* name: DeleteAuthor :exec */
DELETE FROM authors
WHERE id = ?;

/* name: AuthorsByExecutor :many */
SELECT * FROM authors WHERE name = sqlc.arg(executor) ORDER BY id;

/* name: AuthorsByQ :many */
SELECT * FROM authors WHERE bio = sqlc.arg(q) ORDER BY id;

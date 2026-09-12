/* name: GetAuthor :one */
SELECT * FROM authors
WHERE id = ? LIMIT 1;

/* name: ListAuthors :many */
SELECT * FROM authors
ORDER BY name;

-- name: CountAuthors :one
SELECT COUNT(*) FROM authors;

/* name: CreateAuthor :execlastid */
INSERT INTO authors (
  name, bio
) VALUES (
  ?, ?
);

/* name: DeleteAuthor :exec */
DELETE FROM authors
WHERE id = ?;

/* name: CreateAuthorWithID :execlastid */
INSERT INTO authors (id, name) VALUES (?, ?);

/* name: CreateAuthorReturningID :execlastid */
INSERT INTO authors (name, bio) VALUES (?, ?) RETURNING id;

/* name: RenameAuthorReturningID :execrows */
UPDATE authors SET name = ? WHERE id = ? RETURNING id;

/* name: DeleteAuthorReturningID :exec */
DELETE FROM authors WHERE id = ? RETURNING id;

/* name: AuthorsByClient :many */
SELECT * FROM authors WHERE name = sqlc.arg(client) ORDER BY id;

/* name: AuthorsByStatement :many */
SELECT * FROM authors WHERE bio = sqlc.arg(statement) ORDER BY id;

/* name: AuthorsByParams :many */
SELECT * FROM authors WHERE name = sqlc.arg(params) ORDER BY id;

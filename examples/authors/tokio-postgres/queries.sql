-- name: GetAuthor :one
SELECT * FROM authors
WHERE id = sqlc.arg('get_author_with') LIMIT 1;

-- name: ListAuthors :many
SELECT * FROM authors
ORDER BY name;

-- name: CountAuthors :one
SELECT COUNT(*) FROM authors;

-- name: CreateAuthor :one
INSERT INTO authors (
          name, bio
) VALUES (
  $1, $2
)
RETURNING *;

-- name: DeleteAuthor :exec
DELETE FROM authors
WHERE id = $1;

-- name: GetKeywordIdent :one
SELECT id, type FROM keyword_idents
WHERE type = sqlc.arg('type') LIMIT 1;

-- name: GetAuthorsByName :one
SELECT * FROM authors
WHERE name = $1;

-- name: CreateAuthorWithID :execresult
INSERT INTO authors (id, name) VALUES ($1, $2);

-- name: TouchAuthors :execresult
UPDATE authors SET bio = bio
WHERE id >= $1;

-- name: RenameAuthorsReturningID :execrows
UPDATE authors SET name = $1
WHERE id >= $2
RETURNING id;

-- name: DeleteAuthorReturningID :exec
DELETE FROM authors
WHERE id = $1
RETURNING id;

-- name: InsertTimestamps :exec
INSERT INTO timestamps (id, timestamp_val, timestamptz_val, nullable_timestamp)
VALUES ($1, $2, $3, $4);

-- name: GetTimestamps :one
SELECT timestamp_val, timestamptz_val, nullable_timestamp
FROM timestamps
WHERE id = $1;

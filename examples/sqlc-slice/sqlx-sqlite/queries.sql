-- name: ListAuthorsByIDs :many
SELECT id, name
FROM authors
WHERE id IN (sqlc.slice('ids'))
ORDER BY id;

-- name: ListAuthorsByTwoIDLists :many
SELECT id, name
FROM authors
WHERE id IN (sqlc.slice('ids'))
   OR id IN (sqlc.slice('backup_ids'))
ORDER BY id;

-- name: ListAuthorsByIDsMixed :many
SELECT id, name
FROM authors
WHERE id IN (sqlc.slice('ids'))
  AND id >= ?
  AND id NOT IN (sqlc.slice('skip_ids'))
  AND name <> ?
ORDER BY id;

-- name: ListAuthorsByNamedIDs :many
SELECT id, name
FROM authors
WHERE id >= @min_id
  AND id IN (sqlc.slice('ids'))
  AND id <= @max_id
ORDER BY id;

-- name: DeleteAuthorsByIDs :exec
DELETE FROM authors
WHERE id IN (sqlc.slice('ids'));

-- name: ListAuthorLabelsByIDs :many
SELECT id, '/*SLICE:ids*/?' AS label
FROM authors
WHERE id IN (sqlc.slice('ids'))
ORDER BY id;

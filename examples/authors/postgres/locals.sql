-- name: ListAuthorsByLocalNames :many
SELECT * FROM authors
WHERE name = @client OR name = @statement OR name = @values
ORDER BY id;

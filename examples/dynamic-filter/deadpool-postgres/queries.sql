-- name: SearchUsers :many
SELECT id, email, phone
FROM users
WHERE TRUE
  AND email = @email -- :if @email
  AND phone = @phone -- :if @phone
  AND EXISTS ( -- :if @has_orders
    SELECT 1 FROM orders WHERE orders.user_id = users.id
      AND orders.created_at >= @orders_since -- :if @orders_since
  )
  AND id = ANY(@ids::bigint[]) -- :if @ids
ORDER BY
  id ASC,  -- :if @id_asc
  id DESC, -- :if @id_desc
  TRUE
LIMIT sqlc.arg(row_limit)::int;

-- name: CountUsers :one
SELECT COUNT(*) AS total
FROM users
WHERE TRUE
  AND email = @email -- :if @email
  AND users.id = ANY(sqlc.slice('ids')::bigint[]) -- :if @ids
  AND TRUE;

-- name: TouchUsers :execrows
UPDATE users SET phone = phone
WHERE TRUE
  AND email = @email -- :if @email
  AND users.id = ANY(sqlc.slice('ids')::bigint[]) -- :if @ids
  AND TRUE;

-- name: ListAllUsers :many
SELECT id, email, phone
FROM users
ORDER BY id;

-- name: SearchUsersByProfile :many
SELECT id, email, phone
FROM users
WHERE TRUE
  AND email = @email -- :if @email
  AND (profile @> @profile::jsonb OR profile @> @profile::jsonb) -- :if @profile
ORDER BY id;

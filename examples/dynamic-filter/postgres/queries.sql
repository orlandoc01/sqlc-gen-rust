-- name: SearchUsers :many
SELECT id, email, phone
FROM users
WHERE
  email = @email -- :if @email
  AND phone = @phone -- :if @phone
  AND EXISTS ( -- :flag @has_orders
    SELECT 1 FROM orders WHERE orders.user_id = users.id
      AND orders.created_at >= @orders_since -- :if @orders_since
  )
  AND id = ANY(@ids::bigint[]) -- :if @ids
ORDER BY -- :switch @sort id_asc id_desc shortest_email default=id_asc
  id ASC,            -- :case @id_asc
  id DESC,           -- :case @id_desc
  LENGTH(email) ASC, id DESC -- :case @shortest_email
LIMIT sqlc.arg(row_limit)::int;

-- name: CountUsers :one
SELECT COUNT(*) AS total
FROM users
WHERE
  email = @email -- :if @email
  -- :if @ids
  AND users.id = ANY(sqlc.slice('ids')::bigint[])
;

-- name: TouchUsers :execrows
UPDATE users SET phone = phone
WHERE
  email = @email -- :if @email
  -- :if @ids
  AND users.id = ANY(sqlc.slice('ids')::bigint[])
;

-- name: ListAllUsers :many
SELECT id, email, phone
FROM users
ORDER BY id;

-- name: SearchUsersByProfile :many
SELECT id, email, phone
FROM users
WHERE
  email = @email -- :if @email
  AND (profile @> @profile::jsonb OR profile @> @profile::jsonb) -- :if @profile
ORDER BY id;

-- name: SetUserPhone :execrows
UPDATE users SET phone = @new_phone
WHERE
  id = @user_id -- :if @user_id
;

-- name: GetUserByEmail :one
SELECT id, email, phone
FROM users
WHERE
  email = @email -- :if @email
ORDER BY id;

-- name: UpdateUserEmail :exec
UPDATE users SET email = @new_email
WHERE id = @id
  AND email = @email -- :if @email
;

-- name: ListUsersByScope :many
SELECT id, email, phone
FROM users
WHERE -- :switch @scope everyone with_orders without_orders default=everyone
  TRUE -- :case @everyone
  AND EXISTS (SELECT 1 FROM orders WHERE orders.user_id = users.id) -- :case @with_orders
  AND NOT EXISTS (SELECT 1 FROM orders WHERE orders.user_id = users.id) -- :case @without_orders
  AND users.id >= @min_id -- :if @min_id
  AND users.phone <> '' -- :flag @with_phone
ORDER BY -- :switch @order newest oldest default=oldest
  users.id DESC, -- :case @newest
  users.id ASC, -- :case @oldest
  users.email ASC
;


-- name: SearchUsersWithOrders :many
SELECT u.id, u.email, u.phone
FROM users u
JOIN orders o ON o.user_id = u.id AND o.created_at >= @orders_since -- :if @orders_since
WHERE
  u.phone <> '' -- :flag @with_phone
ORDER BY
  o.created_at DESC, -- :if @orders_since
  u.id ASC
;

-- name: SearchUsersByPattern :many
SELECT id, email, phone
FROM users
WHERE
  id > 0
  AND (
    email LIKE @email_pattern -- :if @email_pattern
    OR phone LIKE @phone_pattern -- :if @phone_pattern
  )
ORDER BY id
;

-- name: SearchUsersByLastOrder :many
SELECT u.id, u.email
FROM users u
  -- :flag @with_orders
JOIN (
  SELECT user_id, max(created_at) AS last_order_at
  FROM orders
  GROUP BY user_id
) o ON o.user_id = u.id
WHERE
  u.id > 0
  AND last_order_at >= '2024-06-01' -- :flag @with_orders
ORDER BY
  coalesce(last_order_at, '') DESC, -- :flag @with_orders
  u.id ASC
;

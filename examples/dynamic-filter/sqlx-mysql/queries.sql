-- name: SearchUsers :many
SELECT id, email, phone
FROM users
WHERE
  email = ? -- :if @email
  AND phone = ? -- :if @phone
  AND EXISTS ( -- :flag @has_orders
    SELECT 1 FROM orders WHERE orders.user_id = users.id
      AND orders.created_at >= ? -- :if @created_at
  )
  AND users.id IN (sqlc.slice('ids')) -- :if @ids
ORDER BY -- :switch @sort id_asc id_desc shortest_email default=id_asc
  users.id ASC,            -- :case @id_asc
  users.id DESC,           -- :case @id_desc
  LENGTH(users.email) ASC, users.id DESC -- :case @shortest_email
LIMIT ?;

-- name: CountUsers :one
SELECT COUNT(*) AS total
FROM users
WHERE
  email = ? -- :if @email
  -- :if @ids
  AND users.id IN (sqlc.slice('ids'))
;

-- name: TouchUsers :execrows
UPDATE users SET phone = phone
WHERE
  email = ? -- :if @email
  -- :if @ids
  AND users.id IN (sqlc.slice('ids'))
;

-- name: ListUsersByScope :many
SELECT id, email, phone
FROM users
WHERE -- :switch @scope everyone with_orders without_orders default=everyone
  TRUE -- :case @everyone
  AND EXISTS (SELECT 1 FROM orders WHERE orders.user_id = users.id) -- :case @with_orders
  AND NOT EXISTS (SELECT 1 FROM orders WHERE orders.user_id = users.id) -- :case @without_orders
  AND users.id >= sqlc.arg(min_id) -- :if @min_id
  AND users.phone <> '' -- :flag @with_phone
ORDER BY -- :switch @order newest oldest default=oldest
  users.id DESC, -- :case @newest
  users.id ASC, -- :case @oldest
  users.email ASC
;


-- name: SearchUsersWithOrders :many
SELECT u.id, u.email, u.phone
FROM users u
JOIN orders o ON o.user_id = u.id AND o.created_at >= sqlc.arg(orders_since) -- :if @orders_since
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
    email LIKE sqlc.arg(email_pattern) -- :if @email_pattern
    OR phone LIKE sqlc.arg(phone_pattern) -- :if @phone_pattern
  )
ORDER BY id
;

-- name: ListUsersOrderedByAlias :many
SELECT u.id, u.email AS created_at
FROM users u
JOIN orders o ON o.user_id = u.id -- :flag @with_orders
WHERE u.id > 0
ORDER BY
  coalesce(o.created_at, '') DESC, -- :flag @with_orders
  created_at ASC
;

-- name: SearchUsersByPatternUncached :many
SELECT id, email, phone
FROM users
WHERE
  id > 0
  AND (
    email LIKE sqlc.arg(email_pattern) -- :if @email_pattern
    OR phone LIKE sqlc.arg(phone_pattern) -- :if @phone_pattern
  )
ORDER BY id
;

-- name: SearchUsersWithHashComment :many
SELECT id, email, phone
FROM users
WHERE
  id >= ? # literal ? in a comment
  AND email = ? -- :if @email
ORDER BY id
;

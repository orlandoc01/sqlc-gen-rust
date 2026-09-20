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
  AND users.id IN (sqlc.slice('ids')) -- :if @ids
ORDER BY -- :switch @sort id_asc id_desc shortest_email default=id_asc
  users.id ASC,            -- :case @id_asc
  users.id DESC,           -- :case @id_desc
  LENGTH(users.email) ASC, users.id DESC -- :case @shortest_email
LIMIT sqlc.arg(row_limit);

-- name: CountUsers :one
SELECT COUNT(*) AS total
FROM users
WHERE
  email = @email -- :if @email
  -- :if @ids
  AND users.id IN (sqlc.slice('ids'))
;

-- name: TouchUsers :execrows
UPDATE users SET phone = phone
WHERE
  email = @email -- :if @email
  -- :if @ids
  AND users.id IN (sqlc.slice('ids'))
;

-- name: SearchUsersByEmails :many
SELECT id, email, phone
FROM users
WHERE
  email IN (sqlc.slice('emails')) -- :if @emails
  AND (phone = @contact OR email = @contact) -- :if @contact
ORDER BY id;

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

-- name: CountUsersWithOrders :one
SELECT COUNT(*) AS total
FROM users
WHERE
  EXISTS ( -- :flag @has_orders
    SELECT 1 FROM orders
    WHERE -- :switch @order_age any_age recent default=any_age
      orders.user_id = users.id
      AND TRUE -- :case @any_age
      AND orders.created_at >= '2025-01-01' -- :case @recent
  )
;

-- name: ClearPhones :execrows
UPDATE users SET phone = ''
WHERE -- :switch @target with_orders without_orders default=without_orders
  EXISTS (SELECT 1 FROM orders WHERE orders.user_id = users.id) -- :case @with_orders
  AND NOT EXISTS (SELECT 1 FROM orders WHERE orders.user_id = users.id) -- :case @without_orders
  AND users.id > 0
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

-- name: ListUsersWithoutOrders :many
SELECT u.id, u.email
FROM users u
LEFT JOIN orders o ON o.user_id = u.id -- :flag @check_orders
WHERE
  u.id > 0
  AND o.id IS NULL -- :flag @check_orders
ORDER BY u.id
;

-- name: ListUsersHavingAlias :many
SELECT u.id, u.email AS mail
FROM users u
JOIN orders o ON o.user_id = u.id -- :flag @with_orders
GROUP BY u.id, u.email
HAVING mail <> ''
ORDER BY mail
;

-- name: CountUsersWithPhonedOrders :one
SELECT COUNT(*) AS total
FROM users u
WHERE EXISTS (
  SELECT 1
  FROM orders o
    -- :flag @with_phone
  JOIN users p ON p.id = o.user_id AND p.phone <> ''
  WHERE o.user_id = u.id
)
;

-- sqlc's SQLite engine drops a line comment that ends the statement, so the flag cannot sit on
-- the last line.
-- name: CountPhonedUsers :one
SELECT COUNT(*) AS total
FROM users
WHERE
  phone <> '' -- :flag @with_phone
LIMIT 1
;

-- name: SearchUsersByPatternUncached :many
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

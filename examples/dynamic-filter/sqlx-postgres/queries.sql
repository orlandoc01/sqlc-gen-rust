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
LIMIT sqlc.arg(row_limit);

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

-- name: CountUsersWithJsonKey :one
SELECT COUNT(*) AS total
FROM users
WHERE
  '{"a": 1}'::jsonb ? @key::text
  AND email = @email -- :if @email
  AND '{"b": 1}'::jsonb ?| ARRAY['b', 'c']
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

-- name: CountUsersAtLeastAge :one
SELECT COUNT(*) AS total
FROM users
WHERE
  age >= @min_age -- :if @min_age
;

-- name: SearchUsersByEitherEmail :many
SELECT id, email, phone
FROM users
WHERE
  id > 0
  AND (
    email LIKE @first -- :if @first
    OR email LIKE @second -- :if @second
  )
ORDER BY id
;

-- name: CountUsersByNote :one
SELECT COUNT(*) AS total
FROM users
WHERE
  note = @note -- :if @note
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

-- name: SearchUsersInArray :many
SELECT id, email, phone
FROM users
WHERE
  id = ANY(ARRAY[@first_id::bigint, @second_id::bigint])
  AND email = @email -- :if @email
ORDER BY id
;

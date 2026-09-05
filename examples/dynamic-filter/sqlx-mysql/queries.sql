-- name: SearchUsers :many
SELECT id, email, phone
FROM users
WHERE TRUE
  AND email = ? -- :if @email
  AND phone = ? -- :if @phone
  AND EXISTS ( -- :if @has_orders
    SELECT 1 FROM orders WHERE orders.user_id = users.id
      AND orders.created_at >= ? -- :if @created_at
  )
  AND users.id IN (sqlc.slice('ids')) -- :if @ids
ORDER BY
  users.id ASC,  -- :if @id_asc
  users.id DESC, -- :if @id_desc
  TRUE
LIMIT ?;

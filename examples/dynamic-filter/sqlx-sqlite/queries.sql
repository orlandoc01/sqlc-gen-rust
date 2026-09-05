-- name: SearchUsers :many
SELECT id, email, phone
FROM users
WHERE TRUE
  AND email = @email -- :if @email
  -- :if @phone
  AND phone = @phone
  AND EXISTS ( -- :if @has_orders
    SELECT 1 FROM orders WHERE orders.user_id = users.id
      AND orders.created_at >= @orders_since -- :if @orders_since
  )
  AND users.id IN (sqlc.slice('ids')) -- :if @ids
ORDER BY
  users.id ASC,  -- :if @id_asc
  users.id DESC, -- :if @id_desc
  TRUE
LIMIT sqlc.arg(row_limit);

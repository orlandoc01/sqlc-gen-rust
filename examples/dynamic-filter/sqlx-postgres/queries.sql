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
LIMIT sqlc.arg(row_limit);

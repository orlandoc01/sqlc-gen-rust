-- name: GetMapping :one
SELECT
    bool_val,
    bool_array_val,
    char_val,
    smallint_val,
    int_val,
    int_nullable_val,
    oid_val,
    bigint_val,
    real_val,
    double_val,
    text_val,
    text_nullable_val,
    bytea_val,
    hstore_val,
    timestamp_val,
    timestamptz_val,
    date_val,
    time_val,
    inet_val,
    json_val,
    jsonb_val,
    uuid_val,
    enum_val,
    composite_val
FROM mapping;

-- name: InsertMapping :exec
INSERT INTO mapping (
    bool_val,
    bool_array_val,
    char_val,
    smallint_val,
    int_val,
    int_nullable_val,
    oid_val,
    bigint_val,
    real_val,
    double_val,
    text_val,
    text_nullable_val,
    bytea_val,
    hstore_val,
    timestamp_val,
    timestamptz_val,
    date_val,
    time_val,
    inet_val,
    json_val,
    jsonb_val,
    uuid_val,
    enum_val,
    composite_val
) VALUES (
    $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12,
    $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24
);

-- name: GetByState :many
SELECT id, state FROM state_mappings
WHERE state = sqlc.arg(state)
ORDER BY id;

-- name: GetOneByState :one
SELECT id, state FROM state_mappings
WHERE state = sqlc.arg(state)
LIMIT 1;

-- name: GetByStateWithMinimumID :many
SELECT id, state FROM state_mappings
WHERE state = sqlc.arg(state)
  AND id >= sqlc.arg('minimum_id')
ORDER BY id;

-- name: GetSyncEntry :one
SELECT id, state FROM sync_mappings
WHERE state = sqlc.arg(state);

-- name: SearchSyncEntries :many
SELECT id, state FROM sync_mappings
WHERE TRUE
  AND state = @state -- :if @state
ORDER BY id;

-- name: UpdateKnownTypes :execrows
UPDATE mapping
SET bool_array_val = $1,
    timestamptz_val = $2,
    timestamp_val = $3,
    date_val = $4,
    uuid_val = $5,
    json_val = $6,
    jsonb_val = $7,
    int_val = $8,
    bytea_val = $9,
    text_val = $10,
    bool_val = $11,
    double_val = $12
WHERE id = $13;

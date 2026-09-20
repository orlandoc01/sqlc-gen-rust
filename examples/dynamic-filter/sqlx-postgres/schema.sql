CREATE TABLE users (
    id BIGINT PRIMARY KEY,
    email TEXT NOT NULL,
    phone TEXT NOT NULL,
    age INTEGER NOT NULL DEFAULT 0,
    note TEXT NOT NULL DEFAULT ''
);

CREATE TABLE orders (
    id BIGINT PRIMARY KEY,
    user_id BIGINT NOT NULL,
    created_at TEXT NOT NULL
);

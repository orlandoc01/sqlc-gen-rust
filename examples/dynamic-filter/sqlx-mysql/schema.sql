CREATE TABLE users (
    id BIGINT PRIMARY KEY,
    email TEXT NOT NULL,
    phone TEXT NOT NULL
);

CREATE TABLE orders (
    id BIGINT PRIMARY KEY,
    user_id BIGINT NOT NULL,
    created_at TEXT NOT NULL
);

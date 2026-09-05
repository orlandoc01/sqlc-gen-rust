CREATE TABLE authors (
    id   BIGINT PRIMARY KEY,
    name TEXT NOT NULL
);

CREATE TABLE books (
    id        BIGINT PRIMARY KEY,
    author_id BIGINT NOT NULL,
    title     TEXT NOT NULL
);

CREATE TABLE reviews (
    id      BIGINT PRIMARY KEY,
    book_id BIGINT NOT NULL,
    rating  BIGINT NOT NULL
);

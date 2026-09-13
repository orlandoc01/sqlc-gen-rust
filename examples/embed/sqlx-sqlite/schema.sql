CREATE TABLE authors (
    id   INTEGER PRIMARY KEY,
    name TEXT NOT NULL
);

CREATE TABLE books (
    id        INTEGER PRIMARY KEY,
    author_id INTEGER NOT NULL,
    title     TEXT NOT NULL,
    subtitle  TEXT
);

CREATE TABLE reviews (
    id      INTEGER PRIMARY KEY,
    book_id INTEGER NOT NULL,
    rating  INTEGER NOT NULL
);

CREATE TABLE authors (
          id   integer    PRIMARY KEY AUTOINCREMENT,
          name text   NOT NULL,
          bio  text
);

CREATE TABLE keyword_idents (
          id   integer PRIMARY KEY AUTOINCREMENT,
          type text    NOT NULL
);

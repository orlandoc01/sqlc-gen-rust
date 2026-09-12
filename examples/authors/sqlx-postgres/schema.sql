CREATE TABLE authors (
          id   BIGSERIAL PRIMARY KEY,
          name text      NOT NULL,
          bio  text
);

CREATE TABLE keyword_idents (
          id   BIGSERIAL PRIMARY KEY,
          type text      NOT NULL
);

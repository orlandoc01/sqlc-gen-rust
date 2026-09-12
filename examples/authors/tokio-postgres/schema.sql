CREATE TABLE authors (
          id   BIGSERIAL PRIMARY KEY,
          name text      NOT NULL,
          bio  text
);

CREATE TABLE keyword_idents (
           id   BIGSERIAL PRIMARY KEY,
           type text      NOT NULL
);

CREATE TABLE timestamps (
           id                 BIGINT PRIMARY KEY,
           timestamp_val      TIMESTAMP NOT NULL,
           timestamptz_val    TIMESTAMPTZ NOT NULL,
           nullable_timestamp TIMESTAMP
);

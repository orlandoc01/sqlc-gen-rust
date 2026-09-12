use crate::query::{DbTypeMap, SimpleTypeMap};

use super::sqlx;

// https://github.com/sqlc-dev/sqlc/blob/v1.29.0/internal/codegen/golang/postgresql_type.go#L37-L605
// https://docs.rs/sqlx/latest/sqlx/postgres/types/index.html
pub(crate) const COPY_CHEAP: &[(&str, &[&str])] = &[
    ("i8", &["char"]),
    ("i16", &["smallint", "int2", "pg_catalog.int2"]),
    ("i32", &["serial", "serial4", "pg_catalog.serial4"]),
    ("i64", &["bigserial", "serial8", "pg_catalog.serial8"]),
    ("i16", &["smallserial", "serial2", "pg_catalog.serial2"]),
    ("i32", &["integer", "int", "int4", "pg_catalog.int4"]),
    ("i64", &["bigint", "int8", "pg_catalog.int8"]),
    (
        "f64",
        &["float", "double precision", "float8", "pg_catalog.float8"],
    ),
    ("f32", &["real", "float4", "pg_catalog.float4"]),
    ("bool", &["boolean", "bool", "pg_catalog.bool"]),
    ("uuid::Uuid", &["uuid"]),
];

// TODO: Add PgRange<T>
// https://github.com/sqlc-dev/sqlc/blob/v1.29.0/internal/codegen/golang/postgresql_type.go#L355-L461
pub(crate) const DEFAULT: &[(&str, Option<&str>, &[&str])] = &[
    (
        "String",
        Some("str"),
        &[
            "text",
            "pg_catalog.varchar",
            "pg_catalog.bpchar",
            "string",
            "citext",
            "name",
        ],
    ),
    (
        "Vec<u8>",
        Some("[u8]"),
        &["bytea", "blob", "pg_catalog.bytea"],
    ),
    ("std::net::IpAddr", None, &["inet"]),
    (
        "serde_json::Value",
        None,
        &["json", "pg_catalog.json", "jsonb", "pg_catalog.jsonb"],
    ),
];

pub(crate) fn type_map(
    extra_copy_cheap: &[(&str, &[&str])],
    extra_defaults: &[(&str, Option<&str>, &[&str])],
) -> DbTypeMap {
    let copy_cheap = COPY_CHEAP
        .iter()
        .chain(extra_copy_cheap)
        .copied()
        .collect::<Vec<_>>();
    let defaults = DEFAULT
        .iter()
        .chain(extra_defaults)
        .copied()
        .collect::<Vec<_>>();
    sqlx::type_map(Box::new(SimpleTypeMap::default()), &copy_cheap, &defaults)
}

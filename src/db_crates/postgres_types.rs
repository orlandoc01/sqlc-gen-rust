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

pub(crate) fn type_ident(db_type: &str, dimensions: usize) -> Option<syn::Ident> {
    let lowercase = db_type.to_ascii_lowercase();
    let name = lowercase.strip_prefix("pg_catalog.").unwrap_or(&lowercase);
    let (scalar, array) = match name {
        "boolean" | "bool" => ("BOOL", Some("BOOL_ARRAY")),
        "bytea" => ("BYTEA", Some("BYTEA_ARRAY")),
        "char" | "\"char\"" => ("CHAR", Some("CHAR_ARRAY")),
        "name" => ("NAME", Some("NAME_ARRAY")),
        "bigint" | "int8" | "bigserial" | "serial8" => ("INT8", Some("INT8_ARRAY")),
        "smallint" | "int2" | "smallserial" | "serial2" => ("INT2", Some("INT2_ARRAY")),
        "int2vector" => ("INT2_VECTOR", Some("INT2_VECTOR_ARRAY")),
        "integer" | "int" | "int4" | "serial" | "serial4" => ("INT4", Some("INT4_ARRAY")),
        "regproc" => ("REGPROC", Some("REGPROC_ARRAY")),
        "text" => ("TEXT", Some("TEXT_ARRAY")),
        "oid" => ("OID", Some("OID_ARRAY")),
        "tid" => ("TID", Some("TID_ARRAY")),
        "xid" => ("XID", Some("XID_ARRAY")),
        "cid" => ("CID", Some("CID_ARRAY")),
        "oidvector" => ("OID_VECTOR", Some("OID_VECTOR_ARRAY")),
        "json" => ("JSON", Some("JSON_ARRAY")),
        "xml" => ("XML", Some("XML_ARRAY")),
        "xid8" => ("XID8", Some("XID8_ARRAY")),
        "point" => ("POINT", Some("POINT_ARRAY")),
        "lseg" => ("LSEG", Some("LSEG_ARRAY")),
        "path" => ("PATH", Some("PATH_ARRAY")),
        "box" => ("BOX", Some("BOX_ARRAY")),
        "polygon" => ("POLYGON", Some("POLYGON_ARRAY")),
        "line" => ("LINE", Some("LINE_ARRAY")),
        "cidr" => ("CIDR", Some("CIDR_ARRAY")),
        "real" | "float4" => ("FLOAT4", Some("FLOAT4_ARRAY")),
        "double precision" | "float8" | "float" => ("FLOAT8", Some("FLOAT8_ARRAY")),
        "circle" => ("CIRCLE", Some("CIRCLE_ARRAY")),
        "macaddr8" => ("MACADDR8", Some("MACADDR8_ARRAY")),
        "money" => ("MONEY", Some("MONEY_ARRAY")),
        "macaddr" => ("MACADDR", Some("MACADDR_ARRAY")),
        "inet" => ("INET", Some("INET_ARRAY")),
        "bpchar" | "character" => ("BPCHAR", Some("BPCHAR_ARRAY")),
        "character varying" => ("VARCHAR", Some("VARCHAR_ARRAY")),
        "varchar" => ("VARCHAR", Some("VARCHAR_ARRAY")),
        "date" => ("DATE", Some("DATE_ARRAY")),
        "time" | "time without time zone" => ("TIME", Some("TIME_ARRAY")),
        "timestamp" | "timestamp without time zone" => ("TIMESTAMP", Some("TIMESTAMP_ARRAY")),
        "timestamptz" | "timestamp with time zone" => ("TIMESTAMPTZ", Some("TIMESTAMPTZ_ARRAY")),
        "interval" => ("INTERVAL", Some("INTERVAL_ARRAY")),
        "numeric" | "decimal" => ("NUMERIC", Some("NUMERIC_ARRAY")),
        "timetz" | "time with time zone" => ("TIMETZ", Some("TIMETZ_ARRAY")),
        "bit" => ("BIT", Some("BIT_ARRAY")),
        "varbit" | "bit varying" => ("VARBIT", Some("VARBIT_ARRAY")),
        "refcursor" => ("REFCURSOR", Some("REFCURSOR_ARRAY")),
        "regprocedure" => ("REGPROCEDURE", Some("REGPROCEDURE_ARRAY")),
        "regoper" => ("REGOPER", Some("REGOPER_ARRAY")),
        "regoperator" => ("REGOPERATOR", Some("REGOPERATOR_ARRAY")),
        "regclass" => ("REGCLASS", Some("REGCLASS_ARRAY")),
        "regtype" => ("REGTYPE", Some("REGTYPE_ARRAY")),
        "txid_snapshot" => ("TXID_SNAPSHOT", Some("TXID_SNAPSHOT_ARRAY")),
        "uuid" => ("UUID", Some("UUID_ARRAY")),
        "pg_lsn" => ("PG_LSN", Some("PG_LSN_ARRAY")),
        "tsvector" => ("TS_VECTOR", Some("TS_VECTOR_ARRAY")),
        "tsquery" => ("TSQUERY", Some("TSQUERY_ARRAY")),
        "gtsvector" => ("GTS_VECTOR", Some("GTS_VECTOR_ARRAY")),
        "regconfig" => ("REGCONFIG", Some("REGCONFIG_ARRAY")),
        "regdictionary" => ("REGDICTIONARY", Some("REGDICTIONARY_ARRAY")),
        "jsonb" => ("JSONB", Some("JSONB_ARRAY")),
        "int4range" => ("INT4_RANGE", Some("INT4_RANGE_ARRAY")),
        "numrange" => ("NUM_RANGE", Some("NUM_RANGE_ARRAY")),
        "tsrange" => ("TS_RANGE", Some("TS_RANGE_ARRAY")),
        "tstzrange" => ("TSTZ_RANGE", Some("TSTZ_RANGE_ARRAY")),
        "daterange" => ("DATE_RANGE", Some("DATE_RANGE_ARRAY")),
        "int8range" => ("INT8_RANGE", Some("INT8_RANGE_ARRAY")),
        "jsonpath" => ("JSONPATH", Some("JSONPATH_ARRAY")),
        "regnamespace" => ("REGNAMESPACE", Some("REGNAMESPACE_ARRAY")),
        "regrole" => ("REGROLE", Some("REGROLE_ARRAY")),
        "regcollation" => ("REGCOLLATION", Some("REGCOLLATION_ARRAY")),
        "int4multirange" => ("INT4MULTI_RANGE", Some("INT4MULTI_RANGE_ARRAY")),
        "nummultirange" => ("NUMMULTI_RANGE", Some("NUMMULTI_RANGE_ARRAY")),
        "tsmultirange" => ("TSMULTI_RANGE", Some("TSMULTI_RANGE_ARRAY")),
        "tstzmultirange" => ("TSTZMULTI_RANGE", Some("TSTZMULTI_RANGE_ARRAY")),
        "datemultirange" => ("DATEMULTI_RANGE", Some("DATEMULTI_RANGE_ARRAY")),
        "int8multirange" => ("INT8MULTI_RANGE", Some("INT8MULTI_RANGE_ARRAY")),
        "pg_snapshot" => ("PG_SNAPSHOT", Some("PG_SNAPSHOT_ARRAY")),
        _ => return None,
    };
    let ident = if dimensions == 0 { scalar } else { array? };
    Some(quote::format_ident!("{ident}"))
}

#[cfg(test)]
mod tests {
    use super::type_ident;

    #[test]
    fn maps_postgres_type_names_and_arrays() {
        assert_eq!(type_ident("int4", 0).unwrap().to_string(), "INT4");
        assert_eq!(
            type_ident("pg_catalog.int4", 1).unwrap().to_string(),
            "INT4_ARRAY"
        );
        assert_eq!(type_ident("INT4", 0).unwrap().to_string(), "INT4");
        assert_eq!(type_ident("decimal", 0).unwrap().to_string(), "NUMERIC");
        assert_eq!(
            type_ident("character varying", 0).unwrap().to_string(),
            "VARCHAR"
        );
        assert_eq!(type_ident("any", 0), None);
        assert_eq!(type_ident("pg_brin_bloom_summary", 0), None);
        assert_eq!(type_ident("citext", 0), None);
    }
}

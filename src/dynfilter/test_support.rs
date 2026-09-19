//! Fixtures shared by the dynfilter test modules: a small catalog, parameter numbering, and
//! compilation of a parsed query through the generated runtime.

use super::{
    Dialect, DynFilterInfo,
    resolve::{Catalog, CatalogTable},
    strict,
};
use crate::dynfilter_runtime::{Compiled, Placeholders, compile_with_arg_order};

pub(super) fn table<'a>(schema: &'a str, name: &'a str, columns: &[&'a str]) -> CatalogTable<'a> {
    CatalogTable {
        schema,
        name,
        columns: columns.to_vec(),
    }
}

/// `public.orders(id, user_id, created_at)` and `public.users(id, email)`.
pub(super) fn catalog() -> Catalog<'static> {
    Catalog {
        tables: vec![
            table("public", "orders", &["id", "user_id", "created_at"]),
            table("public", "users", &["id", "email"]),
        ],
    }
}

/// Numbers `names` 1.. in order, the way sqlc reports positional parameters.
pub(super) fn params(names: &[&str]) -> Vec<(String, usize)> {
    names
        .iter()
        .enumerate()
        .map(|(index, name)| (name.to_string(), index + 1))
        .collect()
}

/// Parses against an empty catalog: every table's columns are unknown.
pub(super) fn parse_uncatalogued(
    sql: &str,
    params: &[(String, usize)],
    dialect: Dialect,
) -> Result<Option<DynFilterInfo>, String> {
    strict::parse(sql, params, dialect, &Catalog::default())
}

/// Parses against `catalog()`.
pub(super) fn parse_catalogued(
    sql: &str,
    params: &[(String, usize)],
    dialect: Dialect,
) -> Result<Option<DynFilterInfo>, String> {
    strict::parse(sql, params, dialect, &catalog())
}

pub(super) fn compile(
    info: &DynFilterInfo,
    placeholders: Placeholders,
    arg_order: &[usize],
) -> Compiled {
    compile_with_arg_order(
        &info.annotated_sql,
        placeholders,
        arg_order,
        &info.runtime_plan(),
    )
}

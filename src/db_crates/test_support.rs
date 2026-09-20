use crate::db_crates::GenerateOptions;
use crate::dynfilter::{DynFilters, prepared::Prepared};
use crate::plugin;
use crate::query::{DbTypeMap, Query, QueryError, ReturnRowAttributes, ReturningRows};

pub(crate) fn identifier(name: &str) -> plugin::Identifier {
    plugin::Identifier {
        name: name.to_string(),
        schema: String::new(),
        catalog: String::new(),
    }
}

pub(crate) fn column(name: &str, sqlc_slice: bool) -> plugin::Column {
    plugin::Column {
        name: name.to_string(),
        table: None,
        not_null: true,
        is_array: false,
        comment: String::new(),
        length: 0,
        is_named_param: false,
        is_func_call: false,
        scope: String::new(),
        table_alias: String::new(),
        r#type: Some(identifier("integer")),
        is_sqlc_slice: sqlc_slice,
        embed_table: None,
        original_name: String::new(),
        unsigned: false,
        array_dims: 0,
    }
}

pub(crate) fn query(
    name: &str,
    cmd: &str,
    text: &str,
    columns: Vec<plugin::Column>,
    params: Vec<(i32, plugin::Column)>,
) -> plugin::Query {
    plugin::Query {
        text: text.to_string(),
        name: name.to_string(),
        cmd: cmd.to_string(),
        columns,
        params: params
            .into_iter()
            .map(|(number, column)| plugin::Parameter {
                number,
                column: Some(column),
            })
            .collect(),
        comments: Vec::new(),
        filename: String::new(),
        insert_into_table: None,
    }
}

pub(crate) fn dialect(backend: super::DbCrate) -> crate::dynfilter::Dialect {
    use super::{DbCrate, Sqlx};
    match backend {
        DbCrate::Sqlx(Sqlx::Postgres) | DbCrate::Postgres(_) => {
            crate::dynfilter::Dialect::PostgreSql
        }
        DbCrate::Sqlx(Sqlx::MySql) => crate::dynfilter::Dialect::MySql,
        DbCrate::Sqlx(Sqlx::Sqlite) | DbCrate::Rusqlite => crate::dynfilter::Dialect::Sqlite,
    }
}

/// Rows and parsed queries the way `generate` builds them, resolving against the same catalog.
pub(crate) fn parsed(
    backend: super::DbCrate,
    type_map: &DbTypeMap,
    catalog: Option<&plugin::Catalog>,
    queries: &[plugin::Query],
) -> (Vec<ReturningRows>, Vec<Query>) {
    let resolve_catalog = catalog
        .map(crate::dynfilter::resolve::Catalog::from_plugin)
        .unwrap_or_default();
    queries
        .iter()
        .map(|query| {
            let row = ReturningRows::from_query(
                type_map,
                &ReturnRowAttributes::default(),
                catalog,
                query,
            )
            .unwrap();
            let query = Query::parse(
                type_map,
                query,
                dialect(backend),
                &resolve_catalog,
                backend.apply_static_slices(),
            )
            .unwrap();
            (row, query)
        })
        .unzip()
}

/// Generates the queries the way `generate` does; `dynfilters` turns `dynfilters.prepared` on
/// with those options.
pub(crate) fn generate(
    backend: super::DbCrate,
    type_map: &DbTypeMap,
    catalog: Option<&plugin::Catalog>,
    queries: &[plugin::Query],
    query_parameter_limit: usize,
    dynfilters: Option<&DynFilters>,
) -> Result<proc_macro2::TokenStream, QueryError> {
    let (rows, queries) = parsed(backend, type_map, catalog, queries);
    let prepared = dynfilters
        .map(|dynfilters| Prepared::enumerate(&queries, backend, dynfilters))
        .transpose()?;
    backend.generate_queries(
        &rows,
        &queries,
        &GenerateOptions {
            query_parameter_limit,
            prepared: prepared.as_ref(),
        },
    )
}

use crate::query::{self, Annotation, EmbeddedTable, ReturningRows};

mod params_common;
mod params_common_types;
mod postgres_types;
mod rusqlite;
mod rusqlite_params;
mod sqlx;
mod sqlx_params;
mod tokio_postgres;
mod tokio_postgres_params;

#[cfg(test)]
mod rusqlite_tests;
#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tokio_postgres_name_tests;
#[cfg(test)]
mod tokio_postgres_tests;

pub(crate) use sqlx::Sqlx;
pub(crate) use tokio_postgres::TokioPostgres;

#[derive(Debug, Clone, Copy)]
pub(crate) enum DbCrate {
    Sqlx(Sqlx),
    Rusqlite,
    TokioPostgres,
}

impl Default for DbCrate {
    fn default() -> Self {
        Self::Sqlx(Sqlx::default())
    }
}

impl std::fmt::Display for DbCrate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

impl<'de> serde::Deserialize<'de> for DbCrate {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::ALL
            .into_iter()
            .find(|db_crate| db_crate.name() == value.trim())
            .ok_or_else(|| {
                let supported = Self::ALL.map(Self::name).join(", ");
                serde::de::Error::custom(format!(
                    "db_crate `{value}` is not supported yet; supported: {supported}"
                ))
            })
    }
}

impl DbCrate {
    const ALL: [Self; 5] = [
        Self::Sqlx(Sqlx::Postgres),
        Self::Sqlx(Sqlx::MySql),
        Self::Sqlx(Sqlx::Sqlite),
        Self::Rusqlite,
        Self::TokioPostgres,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::Sqlx(Sqlx::Postgres) => "sqlx-postgres",
            Self::Sqlx(Sqlx::MySql) => "sqlx-mysql",
            Self::Sqlx(Sqlx::Sqlite) => "sqlx-sqlite",
            Self::Rusqlite => "rusqlite",
            Self::TokioPostgres => "tokio-postgres",
        }
    }

    pub(crate) fn db_type_map(self) -> crate::query::DbTypeMap {
        match self {
            Self::Sqlx(sqlx) => sqlx.db_type_map(),
            Self::Rusqlite => rusqlite::Rusqlite.db_type_map(),
            Self::TokioPostgres => TokioPostgres.db_type_map(),
        }
    }

    pub(crate) fn init(self) -> proc_macro2::TokenStream {
        match self {
            Self::Sqlx(_) => proc_macro2::TokenStream::new(),
            Self::Rusqlite => rusqlite::Rusqlite.init(),
            Self::TokioPostgres => proc_macro2::TokenStream::new(),
        }
    }

    pub(crate) fn defined_enum(self, enum_type: &query::DbEnum) -> proc_macro2::TokenStream {
        match self {
            Self::Sqlx(sqlx) => sqlx.defined_enum(enum_type),
            Self::Rusqlite => rusqlite::Rusqlite.defined_enum(enum_type),
            Self::TokioPostgres => TokioPostgres.defined_enum(enum_type),
        }
    }

    pub(crate) fn apply_static_slices(self) -> bool {
        matches!(
            self,
            Self::Sqlx(Sqlx::MySql | Sqlx::Sqlite) | Self::Rusqlite
        )
    }

    pub(crate) fn supports(self, annotation: Annotation) -> bool {
        !matches!(
            (self, annotation),
            (
                _,
                Annotation::CopyFrom
                    | Annotation::BatchExec
                    | Annotation::BatchMany
                    | Annotation::BatchOne
            ) | (
                Self::Sqlx(Sqlx::Postgres) | Self::TokioPostgres,
                Annotation::ExecLastId
            ) | (Self::Rusqlite, Annotation::ExecResult)
        )
    }

    pub(crate) fn generate_queries(
        self,
        rows: &[ReturningRows],
        queries: &[query::Query],
        query_parameter_limit: usize,
    ) -> Result<proc_macro2::TokenStream, query::QueryError> {
        match self {
            Self::Sqlx(sqlx) => {
                params_common::generate_queries(&sqlx, rows, queries, query_parameter_limit)
            }
            Self::Rusqlite => params_common::generate_queries(
                &rusqlite::Rusqlite,
                rows,
                queries,
                query_parameter_limit,
            ),
            Self::TokioPostgres => params_common::generate_queries(
                &TokioPostgres,
                rows,
                queries,
                query_parameter_limit,
            ),
        }
    }
}

fn make_return_row(row: &query::ReturningRows) -> proc_macro2::TokenStream {
    let ident = &row.struct_ident();
    make_struct(ident, &row.attributes, &row.fields)
}

fn make_embedded_table(table: &EmbeddedTable) -> proc_macro2::TokenStream {
    make_struct(&table.ident, &table.attributes, &table.fields)
}

pub(crate) fn make_embedded_tables(
    rows: &[ReturningRows],
) -> Result<proc_macro2::TokenStream, query::QueryError> {
    let mut tables = std::collections::BTreeMap::new();
    let mut idents = std::collections::BTreeMap::new();
    for table in rows.iter().flat_map(ReturningRows::embedded_tables) {
        if tables.contains_key(&table.qualified_name) {
            continue;
        }

        let ident = table.ident.to_string();
        if let Some(existing_table) = idents.insert(ident.clone(), &table.qualified_name) {
            return Err(query::QueryError::conflicting_embedded_table(
                existing_table.clone(),
                table.qualified_name.clone(),
                ident,
            ));
        }
        tables.insert(table.qualified_name.clone(), table);
    }

    let tables = tables.values().map(|table| make_embedded_table(table));
    Ok(quote::quote! {#(#tables)*})
}

/// Field initializers for a row struct, decoding each column by its SELECT ordinal. `getter`
/// receives the ordinal literal and yields the backend's `row.get(N)?` expression.
pub(crate) fn row_field_initializers(
    row: &ReturningRows,
    getter: impl Fn(proc_macro2::Literal) -> proc_macro2::TokenStream,
) -> Vec<proc_macro2::TokenStream> {
    let get = |index: usize| getter(proc_macro2::Literal::usize_unsuffixed(index));
    row.fields
        .iter()
        .zip(row.field_ordinals())
        .map(|(field, ordinal)| {
            let field_ident = &field.name;
            match field.embedded_table() {
                None => {
                    let value = get(ordinal.start);
                    quote::quote! { #field_ident: #value }
                }
                Some(table) => {
                    let table_ident = &table.ident;
                    let fields = table.fields.iter().zip(ordinal).map(|(field, index)| {
                        let field_ident = &field.name;
                        let value = get(index);
                        quote::quote! { #field_ident: #value }
                    });
                    quote::quote! { #field_ident: #table_ident { #(#fields,)* } }
                }
            }
        })
        .collect()
}

pub(crate) fn make_enum(
    enum_type: &query::DbEnum,
    backend_derives: proc_macro2::TokenStream,
    type_attribute: impl Fn(&str) -> proc_macro2::TokenStream,
    variant_attribute: impl Fn(&str) -> proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    let derives = &enum_type.derives;
    let fields = enum_type.values.iter().map(|value| {
        let ident = crate::value_ident(value);
        let attribute = variant_attribute(value);
        quote::quote! { #attribute #ident }
    });
    let enum_name = enum_type.ident();
    let attribute = type_attribute(&enum_type.name);
    quote::quote! {
        #[derive(Debug, Clone, Copy, #backend_derives #(, #derives)*)]
        #attribute
        pub enum #enum_name { #(#fields,)* }
    }
}

fn make_struct(
    ident: &syn::Ident,
    attributes: &Option<proc_macro2::TokenStream>,
    column_fields: &[query::ColumnField],
) -> proc_macro2::TokenStream {
    let fields = column_fields.iter().map(|field| {
        let field_name = &field.name;
        let field_typ = field.row_type();
        let attribute = &field.attribute;
        quote::quote! {
            #attribute
            pub #field_name:#field_typ
        }
    });
    quote::quote! {
        #attributes
        pub struct #ident {
            #(#fields,)*
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(table: EmbeddedTable) -> ReturningRows {
        ReturningRows {
            fields: vec![query::ColumnField {
                name: crate::field_ident("users"),
                name_original: syn::LitStr::new("users", proc_macro2::Span::call_site()),
                typ: query::ColumnFieldType::Embed(table),
                attribute: None,
            }],
            query_name: String::new(),
            attributes: None,
        }
    }

    fn embedded_table(qualified_name: &str) -> EmbeddedTable {
        EmbeddedTable {
            qualified_name: qualified_name.to_string(),
            ident: crate::value_ident("users"),
            fields: Vec::new(),
            attributes: None,
        }
    }

    #[test]
    fn rejects_embedded_tables_with_conflicting_struct_idents() {
        let error = make_embedded_tables(&[
            row(embedded_table("first.users")),
            row(embedded_table("second.users")),
        ])
        .unwrap_err();

        assert_eq!(
            error.to_string(),
            "Embedded tables `first.users` and `second.users` both generate Rust struct `Users`"
        );
    }
}

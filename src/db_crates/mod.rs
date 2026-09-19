use crate::query::{self, Annotation, EmbeddedTable, ReturningRows, RsType, TypeMapper};

mod params_common;
mod params_common_types;
mod postgres;
mod postgres_params;
mod postgres_types;
mod rusqlite;
mod rusqlite_params;
mod sqlx;
mod sqlx_params;
mod validation;

#[cfg(test)]
mod postgres_name_tests;
#[cfg(test)]
mod postgres_tests;
#[cfg(test)]
mod prepared_tests;
#[cfg(test)]
mod rusqlite_tests;
#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod validation_tests;

pub(crate) use params_common::GenerateOptions;
pub(crate) use postgres::Postgres;
pub(crate) use sqlx::Sqlx;

/// Populates `map` with the backend's copy-cheap and default Rust types for each database type.
fn type_map(
    mut map: Box<dyn TypeMapper>,
    copy_cheap_types: &[(&str, &[&str])],
    default_types: &[(&str, Option<&str>, &[&str])],
) -> query::DbTypeMap {
    for (owned_type, db_types) in copy_cheap_types {
        let owned_type = syn::parse_str::<syn::Type>(owned_type).expect("Failed to parse type");
        for db_type in *db_types {
            map.insert_db_type(db_type, RsType::new(owned_type.clone(), None, true));
        }
    }
    for (owned_type, slice_type, db_types) in default_types {
        let owned_type = syn::parse_str::<syn::Type>(owned_type).expect("Failed to parse type");
        let slice_type = slice_type
            .map(|typ| syn::parse_str::<syn::Type>(typ).expect("Failed to parse slice type"));
        for db_type in *db_types {
            map.insert_db_type(
                db_type,
                RsType::new(owned_type.clone(), slice_type.clone(), false),
            );
        }
    }
    query::DbTypeMap::from_dyn(map)
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum DbCrate {
    Sqlx(Sqlx),
    Rusqlite,
    Postgres(Postgres),
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
    pub(crate) const ALL: [Self; 7] = [
        Self::Sqlx(Sqlx::Postgres),
        Self::Sqlx(Sqlx::MySql),
        Self::Sqlx(Sqlx::Sqlite),
        Self::Rusqlite,
        Self::Postgres(Postgres::Sync),
        Self::Postgres(Postgres::Tokio),
        Self::Postgres(Postgres::Deadpool),
    ];

    fn name(self) -> &'static str {
        match self {
            Self::Sqlx(Sqlx::Postgres) => "sqlx-postgres",
            Self::Sqlx(Sqlx::MySql) => "sqlx-mysql",
            Self::Sqlx(Sqlx::Sqlite) => "sqlx-sqlite",
            Self::Rusqlite => "rusqlite",
            Self::Postgres(Postgres::Sync) => "postgres",
            Self::Postgres(Postgres::Tokio) => "tokio-postgres",
            Self::Postgres(Postgres::Deadpool) => "deadpool-postgres",
        }
    }

    pub(crate) fn db_type_map(self) -> crate::query::DbTypeMap {
        match self {
            Self::Sqlx(sqlx) => sqlx.db_type_map(),
            Self::Rusqlite => rusqlite::Rusqlite.db_type_map(),
            Self::Postgres(backend) => backend.db_type_map(),
        }
    }

    pub(crate) fn init(self) -> proc_macro2::TokenStream {
        match self {
            Self::Sqlx(_) => proc_macro2::TokenStream::new(),
            Self::Rusqlite => rusqlite::Rusqlite.init(),
            Self::Postgres(_) => proc_macro2::TokenStream::new(),
        }
    }

    pub(crate) fn defined_enum(self, enum_type: &query::DbEnum) -> proc_macro2::TokenStream {
        match self {
            Self::Sqlx(sqlx) => sqlx.defined_enum(enum_type),
            Self::Rusqlite => rusqlite::Rusqlite.defined_enum(enum_type),
            Self::Postgres(backend) => backend.defined_enum(enum_type),
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
                Self::Sqlx(Sqlx::Postgres) | Self::Postgres(_),
                Annotation::ExecLastId
            ) | (Self::Rusqlite, Annotation::ExecResult)
        )
    }

    /// The placeholder form the generated code renders at run time, as the backend's
    /// generator declares it.
    pub(crate) fn runtime_placeholders(self) -> crate::dynfilter_runtime::Placeholders {
        use params_common::ParamsGenerator as _;
        match self {
            Self::Sqlx(sqlx) => sqlx.placeholders(),
            Self::Rusqlite => rusqlite::Rusqlite.placeholders(),
            Self::Postgres(backend) => backend.placeholders(),
        }
    }

    /// The bind-type classifier for backends that prepare by SQL text and must detect one
    /// text binding two Rust types; only SQLx PostgreSQL does.
    pub(crate) fn bind_classes(self) -> Option<crate::dynfilter::variants::BindClasses<'static>> {
        match self {
            Self::Sqlx(Sqlx::Postgres) => Some(&sqlx_params::bind_classes),
            _ => None,
        }
    }

    pub(crate) fn generate_queries(
        self,
        rows: &[ReturningRows],
        queries: &[query::Query],
        options: &GenerateOptions<'_>,
    ) -> Result<proc_macro2::TokenStream, query::QueryError> {
        self.validate_array_dimensions(rows, queries)?;
        match self {
            Self::Sqlx(sqlx) => params_common::generate_queries(&sqlx, rows, queries, options),
            Self::Rusqlite => {
                params_common::generate_queries(&rusqlite::Rusqlite, rows, queries, options)
            }
            Self::Postgres(backend) => {
                params_common::generate_queries(&backend, rows, queries, options)
            }
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
        crate::unique::insert_unique(&mut idents, ident.clone(), &table.qualified_name).map_err(
            |existing_table| {
                query::QueryError::conflicting_embedded_table(
                    existing_table.clone(),
                    table.qualified_name.clone(),
                    ident,
                )
            },
        )?;
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

pub(crate) fn ordinal_from_row(
    row: &ReturningRows,
    row_type: proc_macro2::TokenStream,
    result_type: proc_macro2::TokenStream,
    getter: impl Fn(proc_macro2::Literal) -> proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    let struct_tokens = make_return_row(row);
    let ident = row.struct_ident();
    let fields = row_field_initializers(row, getter);
    quote::quote! {
        #struct_tokens
        impl #ident {
            pub fn from_row(row: &#row_type) -> #result_type {
                Ok(Self { #(#fields,)* })
            }
        }
    }
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

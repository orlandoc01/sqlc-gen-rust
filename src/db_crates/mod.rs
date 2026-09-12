use crate::query::{self, Annotation, EmbeddedTable, ReturningRows};

mod params_common;
mod rusqlite;
mod rusqlite_params;
mod sqlx;
pub(crate) mod sqlx_params;

#[cfg(test)]
mod rusqlite_tests;

pub(crate) use sqlx::Sqlx;

#[derive(Debug, Clone, Copy)]
pub(crate) enum DbCrate {
    Sqlx(Sqlx),
    Rusqlite,
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
    const ALL: [Self; 4] = [
        Self::Sqlx(Sqlx::Postgres),
        Self::Sqlx(Sqlx::MySql),
        Self::Sqlx(Sqlx::Sqlite),
        Self::Rusqlite,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::Sqlx(Sqlx::Postgres) => "sqlx-postgres",
            Self::Sqlx(Sqlx::MySql) => "sqlx-mysql",
            Self::Sqlx(Sqlx::Sqlite) => "sqlx-sqlite",
            Self::Rusqlite => "rusqlite",
        }
    }

    pub(crate) fn db_type_map(self) -> crate::query::DbTypeMap {
        match self {
            Self::Sqlx(sqlx) => sqlx.db_type_map(),
            Self::Rusqlite => rusqlite::Rusqlite.db_type_map(),
        }
    }

    pub(crate) fn init(self) -> proc_macro2::TokenStream {
        match self {
            Self::Sqlx(_) => proc_macro2::TokenStream::new(),
            Self::Rusqlite => rusqlite::Rusqlite.init(),
        }
    }

    pub(crate) fn defined_enum(self, enum_type: &query::DbEnum) -> proc_macro2::TokenStream {
        match self {
            Self::Sqlx(sqlx) => sqlx.defined_enum(enum_type),
            Self::Rusqlite => rusqlite::Rusqlite.defined_enum(enum_type),
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
            ) | (Self::Sqlx(Sqlx::Postgres), Annotation::ExecLastId)
                | (Self::Rusqlite, Annotation::ExecResult)
        )
    }

    pub(crate) fn generate_queries(
        self,
        rows: &[ReturningRows],
        queries: &[query::Query],
        query_parameter_limit: usize,
    ) -> proc_macro2::TokenStream {
        match self {
            Self::Sqlx(sqlx) => {
                sqlx_params::generate_queries(&sqlx, rows, queries, query_parameter_limit)
            }
            Self::Rusqlite => {
                rusqlite_params::generate_queries(rows, queries, query_parameter_limit)
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

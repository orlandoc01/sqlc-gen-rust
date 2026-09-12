use crate::query::{DbEnum, ReturningRows};
use quote::ToTokens as _;

use super::{make_enum, make_return_row, postgres_types, row_field_initializers};

#[derive(Debug, Clone, Copy)]
pub(crate) enum TokioPostgres {
    Tokio,
    Deadpool,
}

const POSTGRES_COPY_CHEAP: &[(&str, &[&str])] = &[("u32", &["oid", "pg_catalog.oid"])];

const POSTGRES_DEFAULT: &[(&str, Option<&str>, &[&str])] = &[
    (
        "std::collections::HashMap<String, Option<String>>",
        None,
        &["hstore"],
    ),
    (
        "std::time::SystemTime",
        None,
        &[
            "timestamp",
            "pg_catalog.timestamp",
            "timestamptz",
            "pg_catalog.timestamptz",
        ],
    ),
];

impl TokioPostgres {
    pub(crate) fn db_type_map(self) -> crate::query::DbTypeMap {
        postgres_types::type_map(POSTGRES_COPY_CHEAP, POSTGRES_DEFAULT)
    }

    pub(crate) fn defined_enum(self, enum_type: &DbEnum) -> proc_macro2::TokenStream {
        make_enum(
            enum_type,
            quote::quote! { postgres_types::ToSql, postgres_types::FromSql },
            |name| quote::quote! { #[postgres(name = #name)] },
            |value| quote::quote! { #[postgres(name = #value)] },
        )
    }

    pub(crate) fn returning_ordinal_row(self, row: &ReturningRows) -> proc_macro2::TokenStream {
        let struct_tokens = make_return_row(row);
        let ident = row.struct_ident();
        let row_ident = quote::format_ident!("row");
        let paths = self.paths();
        let row_type = paths.row;
        let error = paths.error;
        let fields = row_field_initializers(row, |index| {
            quote::quote! { #row_ident.try_get(#index)? }
        });
        quote::quote! {
            #struct_tokens
            impl #ident {
                pub fn from_row(#row_ident: &#row_type) -> Result<Self, #error> {
                    Ok(Self { #(#fields,)* })
                }
            }
        }
    }

    pub(crate) fn paths(self) -> TokioPostgresPaths {
        TokioPostgresPaths {
            client: match self {
                Self::Tokio => quote::quote! {tokio_postgres::GenericClient},
                Self::Deadpool => quote::quote! {deadpool_postgres::GenericClient},
            },
            statement: self.item("Statement"),
            to_statement: self.item("ToStatement"),
            row_stream: self.item("RowStream"),
            row: self.item("Row"),
            error: self.item("Error"),
            to_sql: self.item("types::ToSql"),
        }
    }

    fn item(self, name: &str) -> proc_macro2::TokenStream {
        let root = match self {
            Self::Tokio => "tokio_postgres",
            Self::Deadpool => "deadpool_postgres::tokio_postgres",
        };
        syn::parse_str::<syn::Path>(&format!("{root}::{name}"))
            .expect("valid path")
            .into_token_stream()
    }

    pub(crate) fn prepare_statement(
        self,
        client: &syn::Ident,
        sql: &syn::Ident,
    ) -> proc_macro2::TokenStream {
        match self {
            Self::Tokio => quote::quote! { #client.prepare(#sql).await },
            Self::Deadpool => quote::quote! { #client.prepare_cached(#sql).await },
        }
    }
}

pub(crate) struct TokioPostgresPaths {
    pub(crate) client: proc_macro2::TokenStream,
    pub(crate) statement: proc_macro2::TokenStream,
    pub(crate) to_statement: proc_macro2::TokenStream,
    pub(crate) row_stream: proc_macro2::TokenStream,
    pub(crate) row: proc_macro2::TokenStream,
    pub(crate) error: proc_macro2::TokenStream,
    pub(crate) to_sql: proc_macro2::TokenStream,
}

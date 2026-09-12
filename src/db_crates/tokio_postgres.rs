use crate::query::{DbEnum, ReturningRows};

use super::{make_enum, make_return_row, postgres_types, row_field_initializers};

#[derive(Debug, Clone, Copy)]
pub(crate) struct TokioPostgres;

pub(crate) const TOKIO_POSTGRES_COPY_CHEAP: &[(&str, &[&str])] =
    &[("u32", &["oid", "pg_catalog.oid"])];

pub(crate) const TOKIO_POSTGRES_DEFAULT: &[(&str, Option<&str>, &[&str])] = &[
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
        postgres_types::type_map(TOKIO_POSTGRES_COPY_CHEAP, TOKIO_POSTGRES_DEFAULT)
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
            client: quote::quote! {tokio_postgres::GenericClient},
            statement: quote::quote! {tokio_postgres::Statement},
            to_statement: quote::quote! {tokio_postgres::ToStatement},
            row_stream: quote::quote! {tokio_postgres::RowStream},
            row: quote::quote! {tokio_postgres::Row},
            error: quote::quote! {tokio_postgres::Error},
            to_sql: quote::quote! {tokio_postgres::types::ToSql},
        }
    }

    pub(crate) fn prepare_statement(
        self,
        client: &syn::Ident,
        sql: &syn::Ident,
    ) -> proc_macro2::TokenStream {
        quote::quote! { #client.prepare(#sql).await }
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

use crate::query::DbEnum;
use quote::ToTokens as _;

use super::{make_enum, postgres_types};

#[derive(Debug, Clone, Copy)]
pub(crate) enum Postgres {
    Sync,
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

impl Postgres {
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

    pub(crate) fn paths(self) -> PostgresPaths {
        let (plain, iterator, row_iter) = match self {
            Self::Sync => {
                let lifetime = syn::Lifetime::new("'c", proc_macro2::Span::call_site());
                (
                    ClientSignature {
                        lifetime: proc_macro2::TokenStream::new(),
                        client_ref: quote::quote! {&mut},
                    },
                    ClientSignature {
                        lifetime: quote::quote! {<#lifetime>},
                        client_ref: quote::quote! {&#lifetime mut},
                    },
                    quote::quote! {postgres::RowIter<#lifetime>},
                )
            }
            Self::Tokio | Self::Deadpool => {
                let signature = ClientSignature {
                    lifetime: proc_macro2::TokenStream::new(),
                    client_ref: quote::quote! {&},
                };
                (signature.clone(), signature, self.item("RowStream"))
            }
        };
        PostgresPaths {
            async_token: self.async_token(),
            await_token: self.await_token(),
            plain,
            iterator,
            client: match self {
                Self::Sync => quote::quote! {postgres::GenericClient},
                Self::Tokio => quote::quote! {tokio_postgres::GenericClient},
                Self::Deadpool => quote::quote! {deadpool_postgres::GenericClient},
            },
            statement: self.item("Statement"),
            to_statement: self.item("ToStatement"),
            row_iter,
            row: self.item("Row"),
            error: self.item("Error"),
            to_sql: self.item("types::ToSql"),
        }
    }

    fn item(self, name: &str) -> proc_macro2::TokenStream {
        let root = match self {
            Self::Sync => "postgres",
            Self::Tokio => "tokio_postgres",
            Self::Deadpool => "deadpool_postgres::tokio_postgres",
        };
        syn::parse_str::<syn::Path>(&format!("{root}::{name}"))
            .expect("valid path")
            .into_token_stream()
    }

    fn async_token(self) -> proc_macro2::TokenStream {
        match self {
            Self::Sync => proc_macro2::TokenStream::new(),
            Self::Tokio | Self::Deadpool => quote::quote! {async},
        }
    }

    fn await_token(self) -> proc_macro2::TokenStream {
        match self {
            Self::Sync => proc_macro2::TokenStream::new(),
            Self::Tokio | Self::Deadpool => quote::quote! {.await},
        }
    }

    pub(crate) fn many_iterator_suffix(self) -> &'static str {
        match self {
            Self::Sync => "iter",
            Self::Tokio | Self::Deadpool => "stream",
        }
    }

    pub(crate) fn many_iterator_helpers(self) -> (&'static str, &'static str) {
        match self {
            Self::Sync => ("iter helper", "iter with helper"),
            Self::Tokio | Self::Deadpool => ("stream helper", "stream with helper"),
        }
    }

    pub(crate) fn prepare_statement(
        self,
        client: &syn::Ident,
        sql: &syn::Ident,
    ) -> proc_macro2::TokenStream {
        match self {
            Self::Sync => quote::quote! { #client.prepare(#sql) },
            Self::Tokio => quote::quote! { #client.prepare(#sql).await },
            Self::Deadpool => quote::quote! { #client.prepare_cached(#sql).await },
        }
    }
}

#[derive(Clone)]
pub(crate) struct ClientSignature {
    pub(crate) lifetime: proc_macro2::TokenStream,
    pub(crate) client_ref: proc_macro2::TokenStream,
}

pub(crate) struct PostgresPaths {
    pub(crate) async_token: proc_macro2::TokenStream,
    pub(crate) await_token: proc_macro2::TokenStream,
    pub(crate) plain: ClientSignature,
    pub(crate) iterator: ClientSignature,
    pub(crate) client: proc_macro2::TokenStream,
    pub(crate) statement: proc_macro2::TokenStream,
    pub(crate) to_statement: proc_macro2::TokenStream,
    pub(crate) row_iter: proc_macro2::TokenStream,
    pub(crate) row: proc_macro2::TokenStream,
    pub(crate) error: proc_macro2::TokenStream,
    pub(crate) to_sql: proc_macro2::TokenStream,
}

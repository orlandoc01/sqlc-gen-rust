use crate::dynfilter_runtime::{Dialect, Placeholders};
use crate::query::{Annotation, Query, ReturningRows};

use super::{
    params_common::{self, ParameterAccess, ParamsGenerator, PreparedParts, QueryParts, Warmup},
    postgres::{Postgres, PostgresPaths},
};

mod dynamic;
mod functions;
mod names;

impl ParamsGenerator for Postgres {
    fn placeholders(&self) -> Placeholders {
        Placeholders::Numbered
    }

    fn dialect(&self) -> Dialect {
        Dialect::Postgres
    }

    fn returning_row(&self, row: &ReturningRows) -> proc_macro2::TokenStream {
        let paths = self.paths();
        let row_type = paths.row;
        let error = paths.error;
        super::ordinal_from_row(
            row,
            row_type,
            quote::quote! {Result<Self, #error>},
            |index| quote::quote! {row.try_get(#index)?},
        )
    }

    fn generated_functions(&self, query: &Query) -> Vec<params_common::GeneratedItem> {
        names::generated_functions(*self, query)
    }

    fn query_functions(
        &self,
        query: &Query,
        row: &ReturningRows,
        parts: &QueryParts<'_>,
    ) -> proc_macro2::TokenStream {
        Function::new(*self, query, row, parts).generate()
    }

    /// Only deadpool keeps a statement cache; `postgres::Client` and `tokio_postgres::Client`
    /// would need a statement-passing API, so they get variant constants only.
    fn caches_statements(&self) -> bool {
        matches!(self, Self::Deadpool)
    }

    fn warmup(&self) -> Warmup {
        Warmup {
            param: "client",
            client: quote::quote! {&deadpool_postgres::ClientWrapper},
            result: quote::quote! {Result<(), deadpool_postgres::tokio_postgres::Error>},
            asynchronous: true,
        }
    }

    fn warmup_body(&self, _query: &Query, parts: &PreparedParts<'_>) -> proc_macro2::TokenStream {
        self.warmup().prepare_each(
            parts,
            |client| quote::quote! { #client.prepare_cached(sql) },
        )
    }
}

struct Function<'a> {
    backend: Postgres,
    query: &'a Query,
    row: &'a ReturningRows,
    parts: &'a QueryParts<'a>,
    paths: PostgresPaths,
    name: syn::Ident,
    client: syn::Ident,
}

struct StaticParts {
    statement: syn::Ident,
    values_ident: syn::Ident,
    values: proc_macro2::TokenStream,
    forwarded: proc_macro2::TokenStream,
}

impl<'a> Function<'a> {
    fn new(
        backend: Postgres,
        query: &'a Query,
        row: &'a ReturningRows,
        parts: &'a QueryParts<'a>,
    ) -> Self {
        let name = params_common::query_function_ident(query);
        let client = parts.local("client");
        let paths = backend.paths();

        Self {
            backend,
            query,
            row,
            parts,
            paths,
            name,
            client,
        }
    }

    fn generate(&self) -> proc_macro2::TokenStream {
        if self.query.dynfilter().is_some() {
            return dynamic::functions(self);
        }

        match self.query.annotation {
            Annotation::One => self.one_functions(),
            Annotation::Many => self.many_functions(),
            Annotation::Exec => {
                self.execute_functions(quote::quote! {()}, quote::quote! {.map(|_| ())})
            }
            Annotation::ExecRows | Annotation::ExecResult => {
                self.execute_functions(quote::quote! {u64}, proc_macro2::TokenStream::new())
            }
            Annotation::ExecLastId
            | Annotation::BatchExec
            | Annotation::BatchMany
            | Annotation::BatchOne
            | Annotation::CopyFrom => proc_macro2::TokenStream::new(),
        }
    }

    fn static_parts(&self) -> StaticParts {
        let values_ident = self.parts.local("values");
        StaticParts {
            statement: self.parts.local("statement"),
            values: Self::static_values(self.query, self.parts, &values_ident, &self.paths.to_sql),
            forwarded: Self::forwarded_args(self.query, self.parts),
            values_ident,
        }
    }

    fn static_values(
        query: &Query,
        parts: &QueryParts,
        values: &syn::Ident,
        to_sql: &proc_macro2::TokenStream,
    ) -> proc_macro2::TokenStream {
        let parameters = query.fields.iter().map(|field| {
            let name = &field.name;
            let value = parts.access.field(name);
            quote::quote! {&#value}
        });
        quote::quote! {
            let #values: &[&(dyn #to_sql + ::std::marker::Sync)] = &[#(#parameters,)*];
        }
    }

    fn forwarded_args(query: &Query, parts: &QueryParts) -> proc_macro2::TokenStream {
        match parts.access {
            ParameterAccess::Direct => query
                .fields
                .iter()
                .map(|field| {
                    let name = &field.name;
                    quote::quote! {, #name}
                })
                .collect(),
            ParameterAccess::Struct => quote::quote! {, params},
        }
    }
}

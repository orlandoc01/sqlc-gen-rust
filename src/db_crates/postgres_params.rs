use crate::query::{Annotation, Query, ReturningRows};

use super::{
    params_common::{self, ParameterAccess, ParamsGenerator, QueryParts},
    postgres::{Postgres, PostgresPaths},
};

mod dynamic;
mod functions;
mod names;

impl ParamsGenerator for Postgres {
    fn placeholders(&self) -> proc_macro2::TokenStream {
        quote::quote! {dynfilter::Placeholders::Numbered}
    }

    fn returning_row(&self, row: &ReturningRows) -> proc_macro2::TokenStream {
        let struct_tokens = super::make_return_row(row);
        let ident = row.struct_ident();
        let row_ident = quote::format_ident!("row");
        let paths = self.paths();
        let row_type = paths.row;
        let error = paths.error;
        let fields = super::row_field_initializers(row, |index| {
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

    fn generated_functions(&self, query: &Query) -> Vec<params_common::GeneratedFunction> {
        names::generated_functions(*self, query)
    }

    fn query_functions(
        &self,
        query: &Query,
        row: &ReturningRows,
        parts: &QueryParts,
    ) -> proc_macro2::TokenStream {
        Function::new(*self, query, row, parts).generate()
    }
}

struct Function<'a> {
    backend: Postgres,
    query: &'a Query,
    row: &'a ReturningRows,
    parts: &'a QueryParts,
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
        parts: &'a QueryParts,
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
            match parts.access {
                ParameterAccess::Direct => quote::quote! {&#name},
                ParameterAccess::Struct => quote::quote! {&params.#name},
            }
        });
        quote::quote! {
            let #values: &[&(dyn #to_sql + Sync)] = &[#(#parameters,)*];
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

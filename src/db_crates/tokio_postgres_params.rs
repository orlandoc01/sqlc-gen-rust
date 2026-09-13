use crate::query::{Annotation, Query, ReturningRows};

use super::{
    params_common::{self, ParameterAccess, ParamsGenerator, QueryParts},
    tokio_postgres::{TokioPostgres, TokioPostgresPaths},
};

mod dynamic;
mod names;

impl ParamsGenerator for TokioPostgres {
    fn placeholders(&self) -> proc_macro2::TokenStream {
        quote::quote! {dynfilter::Placeholders::Numbered}
    }

    fn returning_row(&self, row: &ReturningRows) -> proc_macro2::TokenStream {
        self.returning_ordinal_row(row)
    }

    fn generated_functions(&self, query: &Query) -> Vec<params_common::GeneratedFunction> {
        names::generated_functions(query)
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
    backend: TokioPostgres,
    query: &'a Query,
    row: &'a ReturningRows,
    parts: &'a QueryParts,
    paths: TokioPostgresPaths,
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
        backend: TokioPostgres,
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

    fn one_functions(&self) -> proc_macro2::TokenStream {
        let parts = self.static_parts();
        let name = &self.name;
        let opt = quote::format_ident!("{name}_opt");
        let with = quote::format_ident!("{name}_with");
        let opt_with = quote::format_ident!("{name}_opt_with");
        let row = self.row.struct_ident();
        let client = &self.client;
        let statement = &parts.statement;
        let values_ident = &parts.values_ident;
        let row_type = quote::quote! {#row};
        let optional_row_type = quote::quote! {Option<#row>};
        let one = self.plain_fn(name, &row_type, &with, &parts.forwarded);
        let one_with = self.with_fn(
            &with,
            &row_type,
            &parts,
            quote::quote! {
                let row = #client.query_one(#statement, #values_ident).await?;
                #row::from_row(&row)
            },
        );
        let optional = self.plain_fn(&opt, &optional_row_type, &opt_with, &parts.forwarded);
        let optional_with = self.with_fn(
            &opt_with,
            &optional_row_type,
            &parts,
            quote::quote! {
                #client.query_opt(#statement, #values_ident).await?.map(|row| #row::from_row(&row)).transpose()
            },
        );
        let prepare = self.prepare_function();
        quote::quote! {
            #prepare
            #one
            #one_with
            #optional
            #optional_with
        }
    }

    fn many_functions(&self) -> proc_macro2::TokenStream {
        let parts = self.static_parts();
        let name = &self.name;
        let with = quote::format_ident!("{name}_with");
        let stream = quote::format_ident!("{name}_stream");
        let stream_with = quote::format_ident!("{name}_stream_with");
        let row = self.row.struct_ident();
        let client = &self.client;
        let statement = &parts.statement;
        let values_ident = &parts.values_ident;
        let many_type = quote::quote! {Vec<#row>};
        let stream_type = &self.paths.row_stream;
        let many = self.plain_fn(name, &many_type, &with, &parts.forwarded);
        let many_with = self.with_fn(
            &with,
            &many_type,
            &parts,
            quote::quote! {
                let rows = #client.query(#statement, #values_ident).await?;
                rows.iter().map(#row::from_row).collect()
            },
        );
        let stream_fn = self.plain_fn(&stream, stream_type, &stream_with, &parts.forwarded);
        let stream_with_fn = self.with_fn(
            &stream_with,
            stream_type,
            &parts,
            quote::quote! {
                #client.query_raw(#statement, #values_ident.iter().copied()).await
            },
        );
        let prepare = self.prepare_function();
        quote::quote! {
            #prepare
            #many
            #many_with
            #stream_fn
            #stream_with_fn
        }
    }

    fn execute_functions(
        &self,
        result: proc_macro2::TokenStream,
        map: proc_macro2::TokenStream,
    ) -> proc_macro2::TokenStream {
        let parts = self.static_parts();
        let name = &self.name;
        let with = quote::format_ident!("{name}_with");
        let client = &self.client;
        let statement = &parts.statement;
        let values_ident = &parts.values_ident;
        let execute = self.plain_fn(name, &result, &with, &parts.forwarded);
        let execute_with = self.with_fn(
            &with,
            &result,
            &parts,
            quote::quote! { #client.execute(#statement, #values_ident).await #map },
        );
        let prepare = self.prepare_function();
        quote::quote! {
            #prepare
            #execute
            #execute_with
        }
    }

    fn plain_fn(
        &self,
        name: &syn::Ident,
        return_type: &proc_macro2::TokenStream,
        with_name: &syn::Ident,
        forwarded: &proc_macro2::TokenStream,
    ) -> proc_macro2::TokenStream {
        let client = &self.client;
        let constant = &self.parts.constant;
        let arguments = &self.parts.arguments;
        let generic_client = &self.paths.client;
        let error = &self.paths.error;
        quote::quote! {
            pub async fn #name(#client: &impl #generic_client #arguments) -> Result<#return_type, #error> {
                self::#with_name(#client, #constant #forwarded).await
            }
        }
    }

    fn with_fn(
        &self,
        name: &syn::Ident,
        return_type: &proc_macro2::TokenStream,
        parts: &StaticParts,
        body: proc_macro2::TokenStream,
    ) -> proc_macro2::TokenStream {
        let client = &self.client;
        let statement = &parts.statement;
        let arguments = &self.parts.arguments;
        let values = &parts.values;
        let generic_client = &self.paths.client;
        let to_statement = &self.paths.to_statement;
        let error = &self.paths.error;
        quote::quote! {
            pub async fn #name(#client: &impl #generic_client, #statement: &(impl #to_statement + ?Sized + Sync + Send) #arguments) -> Result<#return_type, #error> {
                #values
                #body
            }
        }
    }

    fn prepare_function(&self) -> proc_macro2::TokenStream {
        let prepare = quote::format_ident!("prepare_{}", self.name);
        let prepare_statement = self
            .backend
            .prepare_statement(&self.client, &self.parts.constant);
        let client = &self.client;
        let generic_client = &self.paths.client;
        let statement = &self.paths.statement;
        let error = &self.paths.error;
        quote::quote! {
            pub async fn #prepare(#client: &impl #generic_client) -> Result<#statement, #error> {
                #prepare_statement
            }
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

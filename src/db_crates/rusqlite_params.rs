use crate::query::{Annotation, Query, ReturningRows};

use super::{
    params_common::{self, ParameterAccess, ParamsGenerator},
    rusqlite::Rusqlite,
};

pub(crate) fn generate_queries(
    rows: &[ReturningRows],
    queries: &[Query],
    query_parameter_limit: usize,
) -> proc_macro2::TokenStream {
    params_common::generate_queries(&Rusqlite, rows, queries, query_parameter_limit)
}

impl ParamsGenerator for Rusqlite {
    fn placeholders(&self) -> proc_macro2::TokenStream {
        quote::quote! {dynfilter::Placeholders::NumberedSqlite}
    }

    fn returning_row(&self, row: &ReturningRows) -> proc_macro2::TokenStream {
        self.returning_ordinal_row(row)
    }

    fn query_functions(
        &self,
        query: &Query,
        row: &ReturningRows,
        parts: &params_common::QueryParts,
    ) -> proc_macro2::TokenStream {
        query_functions(query, row, parts)
    }
}

fn query_functions(
    query: &Query,
    row: &ReturningRows,
    parts: &params_common::QueryParts,
) -> proc_macro2::TokenStream {
    let function = params_common::query_function_ident(query);
    let arguments = &parts.arguments;
    match query.annotation {
        Annotation::One => {
            let row = row.struct_ident();
            let setup = query_setup(query, parts);
            let opt_function = quote::format_ident!("{function}_opt");
            quote::quote! {
                pub fn #function(client: &impl RusqliteClient #arguments) -> rusqlite::Result<#row> {
                    #setup
                    statement.query_row(params, #row::from_row)
                }

                pub fn #opt_function(client: &impl RusqliteClient #arguments) -> rusqlite::Result<Option<#row>> {
                    use rusqlite::OptionalExtension as _;

                    #setup
                    statement.query_row(params, #row::from_row).optional()
                }
            }
        }
        Annotation::Many => {
            let row = row.struct_ident();
            let setup = query_setup(query, parts);
            quote::quote! {
                pub fn #function(client: &impl RusqliteClient #arguments) -> rusqlite::Result<Vec<#row>> {
                    #setup
                    statement.query_map(params, #row::from_row)?.collect()
                }
            }
        }
        Annotation::Exec => execute_function(
            &function,
            arguments,
            query,
            parts,
            quote::quote! {()},
            quote::quote! {.map(|_| ())},
        ),
        Annotation::ExecRows => execute_function(
            &function,
            arguments,
            query,
            parts,
            quote::quote! {usize},
            proc_macro2::TokenStream::new(),
        ),
        Annotation::ExecLastId => {
            let setup = query_setup(query, parts);
            quote::quote! {
                pub fn #function(client: &impl RusqliteClient #arguments) -> rusqlite::Result<i64> {
                    #setup
                    statement.execute(params)?;
                    Ok(client.connection().last_insert_rowid())
                }
            }
        }
        Annotation::ExecResult
        | Annotation::BatchExec
        | Annotation::BatchMany
        | Annotation::BatchOne
        | Annotation::CopyFrom => proc_macro2::TokenStream::new(),
    }
}

fn execute_function(
    function: &syn::Ident,
    arguments: &proc_macro2::TokenStream,
    query: &Query,
    parts: &params_common::QueryParts,
    return_type: proc_macro2::TokenStream,
    map: proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    let setup = query_setup(query, parts);
    quote::quote! {
        pub fn #function(client: &impl RusqliteClient #arguments) -> rusqlite::Result<#return_type> {
            #setup
            statement.execute(params)#map
        }
    }
}

fn query_setup(query: &Query, parts: &params_common::QueryParts) -> proc_macro2::TokenStream {
    if query.dynfilter().is_some() {
        return dynamic_query_setup(query, &parts.constant);
    }

    let constant = &parts.constant;
    let params = static_params(query, parts.access);
    quote::quote! {
        let mut statement = client.connection().prepare_cached(#constant)?;
        let params = #params;
    }
}

fn static_params(query: &Query, access: ParameterAccess) -> proc_macro2::TokenStream {
    let values = query.fields.iter().map(|field| {
        let name = &field.name;
        match access {
            ParameterAccess::Direct => quote::quote! {#name},
            ParameterAccess::Struct => quote::quote! {params.#name},
        }
    });
    quote::quote! {rusqlite::params![#(#values),*]}
}

fn dynamic_query_setup(query: &Query, constant: &syn::Ident) -> proc_macro2::TokenStream {
    let dynamic = quote::format_ident!("{constant}_DYN");
    let args = params_common::dynamic_args(query);
    let binds = dynamic_binds(query);
    quote::quote! {
        let args = [#(#args,)*];
        let (sql, binds) = #dynamic.build(&args);
        let values = binds
            .into_iter()
            .map(|bind| -> &dyn rusqlite::ToSql {
                match bind {
                    #(#binds)*
                    _ => unreachable!("dynfilter bind plan referenced an unknown argument"),
                }
            })
            .collect::<Vec<_>>();
        let mut statement = client.connection().prepare(&sql)?;
        let params = rusqlite::params_from_iter(values);
    }
}

fn dynamic_binds(query: &Query) -> Vec<proc_macro2::TokenStream> {
    let info = query.dynfilter().expect("dynamic query");
    query
        .fields
        .iter()
        .enumerate()
        .map(|(index, field)| {
            let name = &field.name;
            let arg_index = query.param_number(index) - 1;
            if query.is_sqlc_slice(index) {
                let value = if info
                    .conditional_param_numbers
                    .contains(&query.param_number(index))
                {
                    quote::quote! {&params.#name.unwrap()[element]}
                } else {
                    quote::quote! {&params.#name[element]}
                };
                return quote::quote! {
                    dynfilter::Bind::Elem(#arg_index, element) => #value,
                };
            }

            let value = if info
                .conditional_param_numbers
                .contains(&query.param_number(index))
            {
                quote::quote! {params.#name.as_ref().unwrap()}
            } else {
                quote::quote! {&params.#name}
            };
            quote::quote! {
                dynfilter::Bind::Arg(#arg_index) => #value,
            }
        })
        .collect()
}

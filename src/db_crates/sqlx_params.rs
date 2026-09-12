use super::{
    params_common::{self, ParameterAccess, ParamsGenerator},
    sqlx::Sqlx,
};
use crate::query::{Annotation, Query, ReturningRows};

pub(crate) fn generate_queries(
    sqlx: &Sqlx,
    rows: &[ReturningRows],
    queries: &[Query],
    query_parameter_limit: usize,
) -> proc_macro2::TokenStream {
    params_common::generate_queries(sqlx, rows, queries, query_parameter_limit)
}

impl ParamsGenerator for Sqlx {
    fn placeholders(&self) -> proc_macro2::TokenStream {
        match self {
            Self::MySql => quote::quote! {dynfilter::Placeholders::Question},
            Self::Postgres => quote::quote! {dynfilter::Placeholders::Numbered},
            Self::Sqlite => quote::quote! {dynfilter::Placeholders::NumberedSqlite},
        }
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
        query_functions(self, row, query, parts)
    }
}

fn query_functions(
    sqlx: &Sqlx,
    row: &ReturningRows,
    query: &Query,
    parts: &params_common::QueryParts,
) -> proc_macro2::TokenStream {
    let function = params_common::query_function_ident(query);
    let database = sqlx.database_ident();
    let arguments = &parts.arguments;
    match query.annotation {
        Annotation::One => {
            let row = row.struct_ident();
            let setup = make_query_setup(sqlx, query, parts, Some(&row));
            let opt_function = quote::format_ident!("{function}_opt");
            quote::quote! {
                pub async fn #function<'e>(
                    executor: impl sqlx::Executor<'e, Database = #database>
                    #arguments
                ) -> Result<#row, sqlx::Error> {
                    #setup
                    q.fetch_one(executor).await
                }

                pub async fn #opt_function<'e>(
                    executor: impl sqlx::Executor<'e, Database = #database>
                    #arguments
                ) -> Result<Option<#row>, sqlx::Error> {
                    #setup
                    q.fetch_optional(executor).await
                }
            }
        }
        Annotation::Many => {
            let row = row.struct_ident();
            let setup = make_query_setup(sqlx, query, parts, Some(&row));
            quote::quote! {
                pub async fn #function<'e>(
                    executor: impl sqlx::Executor<'e, Database = #database>
                    #arguments
                ) -> Result<Vec<#row>, sqlx::Error> {
                    #setup
                    q.fetch_all(executor).await
                }
            }
        }
        Annotation::Exec => execute_function(
            sqlx,
            query,
            parts,
            quote::quote! {()},
            quote::quote! {.map(|_| ())},
        ),
        Annotation::ExecRows => execute_function(
            sqlx,
            query,
            parts,
            quote::quote! {u64},
            quote::quote! {.map(|result| result.rows_affected())},
        ),
        Annotation::ExecResult => execute_function(
            sqlx,
            query,
            parts,
            quote::quote! {<#database as sqlx::Database>::QueryResult},
            proc_macro2::TokenStream::new(),
        ),
        Annotation::ExecLastId => last_id_function(sqlx, query, parts),
        Annotation::BatchExec
        | Annotation::BatchMany
        | Annotation::BatchOne
        | Annotation::CopyFrom => proc_macro2::TokenStream::new(),
    }
}

fn execute_function(
    sqlx: &Sqlx,
    query: &Query,
    parts: &params_common::QueryParts,
    return_type: proc_macro2::TokenStream,
    map: proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    let function = params_common::query_function_ident(query);
    let database = sqlx.database_ident();
    let arguments = &parts.arguments;
    let setup = make_query_setup(sqlx, query, parts, None);
    quote::quote! {
        pub async fn #function<'e>(
            executor: impl sqlx::Executor<'e, Database = #database>
            #arguments
        ) -> Result<#return_type, sqlx::Error> {
            #setup
            q.execute(executor).await #map
        }
    }
}

fn last_id_function(
    sqlx: &Sqlx,
    query: &Query,
    parts: &params_common::QueryParts,
) -> proc_macro2::TokenStream {
    let function = params_common::query_function_ident(query);
    let database = sqlx.database_ident();
    let arguments = &parts.arguments;
    let setup = make_query_setup(sqlx, query, parts, None);
    match sqlx {
        Sqlx::Sqlite => quote::quote! {
            pub async fn #function<'e>(
                executor: impl sqlx::Executor<'e, Database = #database>
                #arguments
            ) -> Result<i64, sqlx::Error> {
                #setup
                q.execute(executor).await.map(|result| result.last_insert_rowid())
            }
        },
        Sqlx::MySql => quote::quote! {
            pub async fn #function<'e>(
                executor: impl sqlx::Executor<'e, Database = #database>
                #arguments
            ) -> Result<u64, sqlx::Error> {
                #setup
                q.execute(executor).await.map(|result| result.last_insert_id())
            }
        },
        Sqlx::Postgres => proc_macro2::TokenStream::new(),
    }
}

fn make_query_setup(
    sqlx: &Sqlx,
    query: &Query,
    parts: &params_common::QueryParts,
    row: Option<&syn::Ident>,
) -> proc_macro2::TokenStream {
    if query.dynfilter().is_some() {
        return make_dynamic_query_setup(query, &parts.constant, row);
    }
    let query_ident = quote::format_ident!("q");
    let bind = make_bind(sqlx, query, query_ident.clone(), parts.access);
    let constant = &parts.constant;
    let query = match row {
        Some(row) => quote::quote! {sqlx::query_as::<_, #row>(#constant)},
        None => quote::quote! {sqlx::query(#constant)},
    };
    quote::quote! {
        let #query_ident = #query;
        #bind
    }
}

fn make_dynamic_query_setup(
    query: &Query,
    constant: &syn::Ident,
    row: Option<&syn::Ident>,
) -> proc_macro2::TokenStream {
    let dynamic = quote::format_ident!("{constant}_DYN");
    let args = params_common::dynamic_args(query);
    let binds = dynamic_binds(query);
    let query = match row {
        Some(row) => quote::quote! {sqlx::query_as::<_, #row>(&sql)},
        None => quote::quote! {sqlx::query(&sql)},
    };

    quote::quote! {
        let args = [#(#args,)*];
        let (sql, binds) = #dynamic.build(&args);
        let mut q = #query;
        for bind in binds {
            q = match bind {
                #(#binds)*
                _ => unreachable!("dynfilter bind plan referenced an unknown argument"),
            };
        }
        let q = q.persistent(false);
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
                let elem = if info
                    .conditional_param_numbers
                    .contains(&query.param_number(index))
                {
                    quote::quote! {&params.#name.unwrap()[element]}
                } else {
                    quote::quote! {&params.#name[element]}
                };
                return quote::quote! {
                    dynfilter::Bind::Elem(#arg_index, element) => {
                        let elem = #elem;
                        q.bind(elem)
                    }
                };
            }
            let borrowed = field.scalar_type().need_params_struct_lifetime();
            let value = if info
                .conditional_param_numbers
                .contains(&query.param_number(index))
            {
                if field.scalar_type().copy_cheap() || borrowed {
                    quote::quote! {params.#name.unwrap()}
                } else {
                    quote::quote! {params.#name.as_ref().unwrap()}
                }
            } else if field.scalar_type().copy_cheap() || borrowed {
                quote::quote! {params.#name}
            } else {
                quote::quote! {&params.#name}
            };
            quote::quote! {
                dynfilter::Bind::Arg(#arg_index) => q.bind(#value),
            }
        })
        .collect()
}

fn make_bind(
    sqlx: &Sqlx,
    query: &Query,
    query_ident: syn::Ident,
    access: ParameterAccess,
) -> proc_macro2::TokenStream {
    match access {
        ParameterAccess::Direct => {
            sqlx.query_bind(query, query_ident, |name| quote::quote! {#name})
        }
        ParameterAccess::Struct => {
            sqlx.query_bind(query, query_ident, |name| quote::quote! {params.#name})
        }
    }
}

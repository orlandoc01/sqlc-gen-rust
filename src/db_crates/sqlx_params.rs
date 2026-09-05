use convert_case::{Case, Casing as _};

use super::{QueryAst, sqlx::Sqlx};
use crate::{
    query::{Annotation, Query, ReturningRows},
    value_ident,
};

#[derive(Clone, Copy)]
enum ParameterAccess {
    Direct,
    Struct,
}

pub(super) fn generate_queries(
    sqlx: &Sqlx,
    rows: &[ReturningRows],
    queries: &[Query],
    query_parameter_limit: usize,
    emit_dynamic_filter: bool,
) -> proc_macro2::TokenStream {
    let dynfilter_runtime = (emit_dynamic_filter
        || queries.iter().any(|query| query.dynfilter().is_some()))
    .then(dynfilter_runtime)
    .unwrap_or_default();
    let query_tokens = rows
        .iter()
        .zip(queries)
        .map(|(row, query)| generate_query(sqlx, row, query, query_parameter_limit));
    let query_index = queries.iter().map(|query| {
        let name = syn::LitStr::new(&query.query_name, proc_macro2::Span::call_site());
        let constant = query_const_ident(query);
        quote::quote! {(#name, #constant)}
    });

    quote::quote! {
        #dynfilter_runtime
        #(#query_tokens)*
        pub const QUERIES: &[(&str, &str)] = &[
            #(#query_index,)*
        ];
    }
}

fn dynfilter_runtime() -> proc_macro2::TokenStream {
    let runtime = include_str!("dynfilter_runtime.rs")
        .parse::<proc_macro2::TokenStream>()
        .expect("dynfilter runtime is valid Rust");
    quote::quote! {
        pub mod dynfilter {
            #runtime
        }
    }
}

fn generate_query(
    sqlx: &Sqlx,
    row: &ReturningRows,
    query: &Query,
    query_parameter_limit: usize,
) -> proc_macro2::TokenStream {
    let query_ast = QueryAst::new(query, (*sqlx).into());
    let constant = query_const_ident(query);
    let sql = query.query_str();
    let params = params_definition(&query_ast, query, query_parameter_limit);
    let arguments = function_arguments(query, query_parameter_limit);
    let dynamic = dynamic_static(sqlx, query, &constant);
    let returns = matches!(query.annotation, Annotation::One | Annotation::Many)
        .then(|| sqlx.returning_ordinal_row(row));
    let functions = query_functions(
        sqlx,
        row,
        query,
        &query_ast,
        &constant,
        &arguments,
        query_parameter_limit,
    );

    quote::quote! {
        pub const #constant: &str = #sql;
        #dynamic
        #params
        #returns
        #functions
    }
}

fn dynamic_static(sqlx: &Sqlx, query: &Query, constant: &syn::Ident) -> proc_macro2::TokenStream {
    if query.dynfilter().is_none() {
        return proc_macro2::TokenStream::new();
    }
    let dynamic = quote::format_ident!("{constant}_DYN");
    let placeholders = match sqlx {
        Sqlx::MySql => quote::quote! {dynfilter::Placeholders::Question},
        Sqlx::Postgres => quote::quote! {dynfilter::Placeholders::Numbered},
        Sqlx::Sqlite => quote::quote! {dynfilter::Placeholders::NumberedSqlite},
    };
    let arg_order = query.fields.iter().enumerate().map(|(index, _)| {
        let number = query.param_number(index);
        quote::quote! {#number}
    });
    quote::quote! {
        static #dynamic: std::sync::LazyLock<dynfilter::Compiled> =
            std::sync::LazyLock::new(|| {
                dynfilter::compile_with_arg_order(#constant, #placeholders, &[#(#arg_order,)*])
            });
    }
}

fn query_const_ident(query: &Query) -> syn::Ident {
    query_ident(&query.query_name, Case::UpperSnake)
}

fn query_function_ident(query: &Query) -> syn::Ident {
    query_ident(&query.query_name, Case::Snake)
}

fn query_ident(query_name: &str, case: Case) -> syn::Ident {
    let query_name = crate::normalize_str(query_name);
    let mut name = String::with_capacity(query_name.len());
    let bytes = query_name.as_bytes();
    let mut position = 0;

    while position < bytes.len() {
        let start = position;
        while position < bytes.len() && bytes[position].is_ascii_uppercase() {
            position += 1;
        }

        if position - start >= 2 && bytes.get(position) == Some(&b's') {
            name.push(bytes[start] as char);
            name.extend(
                bytes[start + 1..position]
                    .iter()
                    .map(|byte| (*byte as char).to_ascii_lowercase()),
            );
            name.push('s');
            position += 1;
        } else if start != position {
            name.push_str(&query_name[start..position]);
        } else {
            name.push(bytes[position] as char);
            position += 1;
        }
    }

    quote::format_ident!("{}", name.to_case(case))
}

fn uses_params_struct(query: &Query, query_parameter_limit: usize) -> bool {
    query.dynfilter().is_some() || query.fields.len() > query_parameter_limit
}

fn params_definition(
    query_ast: &QueryAst<'_>,
    query: &Query,
    query_parameter_limit: usize,
) -> proc_macro2::TokenStream {
    if !uses_params_struct(query, query_parameter_limit) {
        return proc_macro2::TokenStream::new();
    }

    let params = params_ident(query);
    let lifetime = &query_ast.lifetime;
    let fields = query.fields.iter().map(|field| {
        let name = &field.name;
        let typ = field.scalar_type().to_params_struct_tokens(Some(lifetime));
        quote::quote! {pub #name: #typ}
    });
    let flags = query.dynfilter().into_iter().flat_map(|info| {
        info.flag_params.iter().map(|flag| {
            let name = crate::field_ident(&flag.name);
            quote::quote! {pub #name: bool}
        })
    });

    if query
        .fields
        .iter()
        .any(|field| field.scalar_type().need_params_struct_lifetime())
    {
        quote::quote! {
            #[derive(Debug, Clone, Default)]
            pub struct #params<#lifetime> {
                #(#fields,)*
                #(#flags,)*
            }
        }
    } else {
        quote::quote! {
            #[derive(Debug, Clone, Default)]
            pub struct #params {
                #(#fields,)*
                #(#flags,)*
            }
        }
    }
}

fn function_arguments(query: &Query, query_parameter_limit: usize) -> proc_macro2::TokenStream {
    if uses_params_struct(query, query_parameter_limit) {
        let params = params_ident(query);
        let params = if query
            .fields
            .iter()
            .any(|field| field.scalar_type().need_params_struct_lifetime())
        {
            quote::quote! {#params<'_>}
        } else {
            quote::quote! {#params}
        };
        return quote::quote! {, params: #params};
    }

    let fields = query.fields.iter().map(|field| {
        let name = &field.name;
        let typ = field.scalar_type().to_params_struct_tokens(None);
        quote::quote! {#name: #typ}
    });
    let fields = quote::quote! {#(#fields),*};

    if query.fields.is_empty() {
        proc_macro2::TokenStream::new()
    } else {
        quote::quote! {, #fields}
    }
}

fn params_ident(query: &Query) -> syn::Ident {
    value_ident(&format!("{}Params", query.query_name))
}

fn query_functions(
    sqlx: &Sqlx,
    row: &ReturningRows,
    query: &Query,
    query_ast: &QueryAst<'_>,
    constant: &syn::Ident,
    arguments: &proc_macro2::TokenStream,
    query_parameter_limit: usize,
) -> proc_macro2::TokenStream {
    let function = query_function_ident(query);
    let database = sqlx.database_ident();
    let access = if uses_params_struct(query, query_parameter_limit) {
        ParameterAccess::Struct
    } else {
        ParameterAccess::Direct
    };

    match query.annotation {
        Annotation::One => {
            let row = row.struct_ident();
            let setup = make_query_setup(sqlx, query, query_ast, constant, access, Some(&row));
            let opt_function = quote::format_ident!("{}_opt", function);
            let opt_query = make_query_setup(sqlx, query, query_ast, constant, access, Some(&row));
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
                    #opt_query
                    q.fetch_optional(executor).await
                }
            }
        }
        Annotation::Many => {
            let row = row.struct_ident();
            let setup = make_query_setup(sqlx, query, query_ast, constant, access, Some(&row));
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
        Annotation::Exec => {
            let setup = make_query_setup(sqlx, query, query_ast, constant, access, None);
            quote::quote! {
                pub async fn #function<'e>(
                    executor: impl sqlx::Executor<'e, Database = #database>
                    #arguments
                ) -> Result<(), sqlx::Error> {
                    #setup
                    q.execute(executor).await.map(|_| ())
                }
            }
        }
        Annotation::ExecRows => {
            let setup = make_query_setup(sqlx, query, query_ast, constant, access, None);
            quote::quote! {
                pub async fn #function<'e>(
                    executor: impl sqlx::Executor<'e, Database = #database>
                    #arguments
                ) -> Result<u64, sqlx::Error> {
                    #setup
                    q.execute(executor)
                        .await
                        .map(|result| result.rows_affected())
                }
            }
        }
        Annotation::ExecResult => {
            let setup = make_query_setup(sqlx, query, query_ast, constant, access, None);
            quote::quote! {
                pub async fn #function<'e>(
                    executor: impl sqlx::Executor<'e, Database = #database>
                    #arguments
                ) -> Result<<#database as sqlx::Database>::QueryResult, sqlx::Error> {
                    #setup
                    q.execute(executor).await
                }
            }
        }
        Annotation::ExecLastId => {
            let setup = make_query_setup(sqlx, query, query_ast, constant, access, None);
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
        Annotation::BatchExec
        | Annotation::BatchMany
        | Annotation::BatchOne
        | Annotation::CopyFrom => proc_macro2::TokenStream::new(),
    }
}

fn make_query_setup(
    sqlx: &Sqlx,
    query: &Query,
    query_ast: &QueryAst<'_>,
    constant: &syn::Ident,
    access: ParameterAccess,
    row: Option<&syn::Ident>,
) -> proc_macro2::TokenStream {
    if query.dynfilter().is_some() {
        return make_dynamic_query_setup(query, constant, row);
    }
    let query_ident = quote::format_ident!("q");
    let sql_ident = quote::format_ident!("sql");
    let bind = make_bind(sqlx, query, query_ident.clone(), access);
    let cache = query_ast
        .need_expand_query()
        .then(|| quote::quote! {let q = q.persistent(false);});
    let query = |sql| match row {
        Some(row) => quote::quote! {sqlx::query_as::<_, #row>(#sql)},
        None => quote::quote! {sqlx::query(#sql)},
    };

    if query_ast.need_expand_query() {
        let expand = make_expand(query_ast, &sql_ident, access);
        let query = query(quote::quote! {&#sql_ident});
        quote::quote! {
            let #sql_ident = #constant;
            #expand
            let #query_ident = #query;
            #bind
            #cache
        }
    } else {
        let query = query(quote::quote! {#constant});
        quote::quote! {
            let #query_ident = #query;
            #bind
        }
    }
}

fn make_dynamic_query_setup(
    query: &Query,
    constant: &syn::Ident,
    row: Option<&syn::Ident>,
) -> proc_macro2::TokenStream {
    let dynamic = quote::format_ident!("{constant}_DYN");
    let args = dynamic_args(query);
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

fn dynamic_args(query: &Query) -> Vec<proc_macro2::TokenStream> {
    let info = query.dynfilter().expect("dynamic query");
    let mut fields = query.fields.iter().enumerate().collect::<Vec<_>>();
    fields.sort_unstable_by_key(|(index, _)| query.param_number(*index));
    let mut args = fields
        .into_iter()
        .map(|(index, field)| {
            let name = &field.name;
            let conditional = info
                .conditional_param_numbers
                .contains(&query.param_number(index));
            if query.is_sqlc_slice(index) {
                if conditional {
                    quote::quote! {dynfilter::Arg::Slice(params.#name.map(<[_]>::len))}
                } else {
                    quote::quote! {dynfilter::Arg::Slice(Some(params.#name.len()))}
                }
            } else if conditional {
                quote::quote! {dynfilter::Arg::from_option(&params.#name)}
            } else {
                quote::quote! {dynfilter::Arg::Active}
            }
        })
        .collect::<Vec<_>>();
    args.extend(info.flag_params.iter().map(|flag| {
        let name = crate::field_ident(&flag.name);
        quote::quote! {dynfilter::Arg::Flag(params.#name)}
    }));
    args
}

fn dynamic_binds(query: &Query) -> Vec<proc_macro2::TokenStream> {
    let info = query.dynfilter().expect("dynamic query");
    query
        .fields
        .iter()
        .enumerate()
        .flat_map(|(index, field)| {
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
                return vec![quote::quote! {
                    dynfilter::Bind::Elem(#arg_index, element) => {
                        let elem = #elem;
                        q.bind(elem)
                    }
                }];
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
            vec![quote::quote! {
                dynfilter::Bind::Arg(#arg_index) => q.bind(#value),
            }]
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

fn make_expand(
    query_ast: &QueryAst<'_>,
    sql_ident: &syn::Ident,
    access: ParameterAccess,
) -> proc_macro2::TokenStream {
    match access {
        ParameterAccess::Direct => {
            query_ast.make_expand_query(sql_ident, |name| quote::quote! {#name})
        }
        ParameterAccess::Struct => {
            query_ast.make_expand_query(sql_ident, |name| quote::quote! {params.#name})
        }
    }
}

#[cfg(test)]
mod tests {
    use convert_case::Case;

    use super::query_ident;

    #[test]
    fn query_identifiers_keep_plural_acronyms_intact() {
        for (query_name, function, constant) in [
            (
                "ListAuthorsByIDs",
                "list_authors_by_ids",
                "LIST_AUTHORS_BY_IDS",
            ),
            (
                "TransactionIDsByFilter",
                "transaction_ids_by_filter",
                "TRANSACTION_IDS_BY_FILTER",
            ),
            (
                "ClearStagedForLLMByIDs",
                "clear_staged_for_llm_by_ids",
                "CLEAR_STAGED_FOR_LLM_BY_IDS",
            ),
            (
                "AccountByExternalID",
                "account_by_external_id",
                "ACCOUNT_BY_EXTERNAL_ID",
            ),
            (
                "DeleteEVMWalletByID",
                "delete_evm_wallet_by_id",
                "DELETE_EVM_WALLET_BY_ID",
            ),
            ("GetAuthor", "get_author", "GET_AUTHOR"),
            (
                "ListAuthorsByTwoIdLists",
                "list_authors_by_two_id_lists",
                "LIST_AUTHORS_BY_TWO_ID_LISTS",
            ),
        ] {
            assert_eq!(query_ident(query_name, Case::Snake).to_string(), function);
            assert_eq!(
                query_ident(query_name, Case::UpperSnake).to_string(),
                constant
            );
        }
    }
}

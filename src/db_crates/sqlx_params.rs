use convert_case::{Case, Casing as _};

use super::{QueryAst, sqlx::Sqlx};
use crate::{
    field_ident,
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
) -> proc_macro2::TokenStream {
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
        #(#query_tokens)*
        pub const QUERIES: &[(&str, &str)] = &[
            #(#query_index,)*
        ];
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
    let returns = query
        .annotation
        .generates_returning_row()
        .then(|| sqlx.returning_row(row));
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
        #params
        #returns
        #functions
    }
}

fn query_const_ident(query: &Query) -> syn::Ident {
    let name = crate::normalize_str(&query.query_name)
        .to_case(Case::Snake)
        .to_ascii_uppercase();
    quote::format_ident!("{name}")
}

fn uses_params_struct(parameter_count: usize, query_parameter_limit: usize) -> bool {
    parameter_count > query_parameter_limit
}

fn params_definition(
    query_ast: &QueryAst<'_>,
    query: &Query,
    query_parameter_limit: usize,
) -> proc_macro2::TokenStream {
    if !uses_params_struct(query.fields.len(), query_parameter_limit) {
        return proc_macro2::TokenStream::new();
    }

    let params = params_ident(query);
    let lifetime = &query_ast.lifetime;
    let fields = query.fields.iter().map(|field| {
        let name = &field.name;
        let typ = field.scalar_type().to_params_struct_tokens(Some(lifetime));
        quote::quote! {pub #name: #typ}
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
            }
        }
    } else {
        quote::quote! {
            #[derive(Debug, Clone, Default)]
            pub struct #params {
                #(#fields,)*
            }
        }
    }
}

fn function_arguments(query: &Query, query_parameter_limit: usize) -> proc_macro2::TokenStream {
    if uses_params_struct(query.fields.len(), query_parameter_limit) {
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

    (!query.fields.is_empty())
        .then(|| quote::quote! {, #fields})
        .unwrap_or_default()
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
    let function = field_ident(&query.query_name);
    let database = sqlx.database_ident();
    let access = uses_params_struct(query.fields.len(), query_parameter_limit)
        .then_some(ParameterAccess::Struct)
        .unwrap_or(ParameterAccess::Direct);

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
                        .map(|result| sqlx::QueryResult::rows_affected(&result))
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
        Annotation::ExecLastId => last_insert_id_function(
            sqlx, query, query_ast, constant, arguments, &function, &database, access,
        ),
        Annotation::BatchExec
        | Annotation::BatchMany
        | Annotation::BatchOne
        | Annotation::CopyFrom => proc_macro2::TokenStream::new(),
    }
}

fn last_insert_id_function(
    sqlx: &Sqlx,
    query: &Query,
    query_ast: &QueryAst<'_>,
    constant: &syn::Ident,
    arguments: &proc_macro2::TokenStream,
    function: &syn::Ident,
    database: &syn::Type,
    access: ParameterAccess,
) -> proc_macro2::TokenStream {
    let query = make_query_setup(sqlx, query, query_ast, constant, access, None);
    match sqlx {
        Sqlx::Sqlite => quote::quote! {
            pub async fn #function<'e>(
                executor: impl sqlx::Executor<'e, Database = #database>
                #arguments
            ) -> Result<i64, sqlx::Error> {
                #query
                q.execute(executor).await.map(|result| result.last_insert_rowid())
            }
        },
        Sqlx::MySql => quote::quote! {
            pub async fn #function<'e>(
                executor: impl sqlx::Executor<'e, Database = #database>
                #arguments
            ) -> Result<u64, sqlx::Error> {
                #query
                q.execute(executor).await.map(|result| result.last_insert_id())
            }
        },
        Sqlx::Postgres => proc_macro2::TokenStream::new(),
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
    use super::uses_params_struct;

    #[test]
    fn parameter_limit_uses_a_struct_only_above_the_limit() {
        assert!(!uses_params_struct(0, 0));
        assert!(uses_params_struct(1, 0));
        assert!(!uses_params_struct(1, 1));
        assert!(uses_params_struct(2, 1));
        assert!(!uses_params_struct(3, 3));
        assert!(uses_params_struct(4, 3));
    }
}

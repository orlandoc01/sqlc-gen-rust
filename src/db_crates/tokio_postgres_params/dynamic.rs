use crate::query::Annotation;

use super::{Function, params_common};

pub(super) fn functions(function: &Function<'_>) -> proc_macro2::TokenStream {
    let paths = &function.paths;
    let name = &function.name;
    let helper = quote::format_ident!("{name}_query");
    let client = &function.client;
    let arguments = &function.parts.arguments;
    let error = &paths.error;
    let generic_client = &paths.client;
    let helper_function = make_helper(function, &helper, &paths.to_sql);
    let setup = quote::quote! { let (sql, values) = #helper(&params); };
    let functions = match function.query.annotation {
        Annotation::One => {
            let row = function.row.struct_ident();
            let opt = quote::format_ident!("{name}_opt");
            quote::quote! {
                pub async fn #name(#client: &impl #generic_client #arguments) -> Result<#row, #error> {
                    #setup
                    let row = #client.query_one(sql.as_str(), &values).await?;
                    #row::from_row(&row)
                }
                pub async fn #opt(#client: &impl #generic_client #arguments) -> Result<Option<#row>, #error> {
                    #setup
                    #client.query_opt(sql.as_str(), &values).await?.map(|row| #row::from_row(&row)).transpose()
                }
            }
        }
        Annotation::Many => {
            let row = function.row.struct_ident();
            let stream = quote::format_ident!("{name}_stream");
            let row_stream = &paths.row_stream;
            quote::quote! {
                pub async fn #name(#client: &impl #generic_client #arguments) -> Result<Vec<#row>, #error> {
                    #setup
                    let rows = #client.query(sql.as_str(), &values).await?;
                    rows.iter().map(#row::from_row).collect()
                }
                pub async fn #stream(#client: &impl #generic_client #arguments) -> Result<#row_stream, #error> {
                    #setup
                    #client.query_raw(sql.as_str(), values).await
                }
            }
        }
        Annotation::Exec => quote::quote! {
            pub async fn #name(#client: &impl #generic_client #arguments) -> Result<(), #error> {
                #setup
                #client.execute(sql.as_str(), &values).await.map(|_| ())
            }
        },
        Annotation::ExecRows | Annotation::ExecResult => quote::quote! {
            pub async fn #name(#client: &impl #generic_client #arguments) -> Result<u64, #error> {
                #setup
                #client.execute(sql.as_str(), &values).await
            }
        },
        Annotation::ExecLastId
        | Annotation::BatchExec
        | Annotation::BatchMany
        | Annotation::BatchOne
        | Annotation::CopyFrom => proc_macro2::TokenStream::new(),
    };
    quote::quote! {
        #helper_function
        #functions
    }
}

fn make_helper(
    function: &Function<'_>,
    helper: &syn::Ident,
    to_sql: &proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    let params = params_common::params_type(function.query);
    let dynamic = quote::format_ident!("{}_DYN", function.parts.constant);
    let args = params_common::dynamic_args(function.query);
    let binds = params_common::dynamic_binds(function.query)
        .into_iter()
        .map(|bind| {
            let pattern = bind.pattern;
            if bind.slice_element.is_some() {
                return quote::quote! {
                    #pattern => unreachable!("tokio-postgres binds sqlc slices as arrays"),
                };
            }
            let name = &bind.field.name;
            quote::quote! { #pattern => &params.#name as _, }
        });
    quote::quote! {
        fn #helper<'p>(params: &'p #params) -> (String, Vec<&'p (dyn #to_sql + Sync)>) {
            let args = [#(#args,)*];
            let (sql, binds) = #dynamic.build(&args);
            let values: Vec<&(dyn #to_sql + Sync)> = binds
                .iter()
                .map(|bind| match bind {
                    #(#binds)*
                    _ => unreachable!("dynfilter bind plan referenced an unknown argument"),
                })
                .collect();
            (sql, values)
        }
    }
}

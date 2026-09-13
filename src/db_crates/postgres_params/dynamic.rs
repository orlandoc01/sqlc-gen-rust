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
    let async_token = &paths.async_token;
    let await_token = &paths.await_token;
    let plain_lifetime = &paths.plain.lifetime;
    let plain_client_ref = &paths.plain.client_ref;
    let iterator_lifetime = &paths.iterator.lifetime;
    let iterator_client_ref = &paths.iterator.client_ref;
    let helper_function = make_helper(function, &helper, &paths.to_sql);
    let setup = quote::quote! { let (sql, values) = #helper(&params); };
    let functions = match function.query.annotation {
        Annotation::One => {
            let row = function.row.struct_ident();
            let opt = quote::format_ident!("{name}_opt");
            quote::quote! {
                pub #async_token fn #name #plain_lifetime(#client: #plain_client_ref impl #generic_client #arguments) -> Result<#row, #error> {
                    #setup
                    let row = #client.query_one(sql.as_str(), &values) #await_token?;
                    #row::from_row(&row)
                }
                pub #async_token fn #opt #plain_lifetime(#client: #plain_client_ref impl #generic_client #arguments) -> Result<Option<#row>, #error> {
                    #setup
                    #client.query_opt(sql.as_str(), &values) #await_token?.map(|row| #row::from_row(&row)).transpose()
                }
            }
        }
        Annotation::Many => {
            let row = function.row.struct_ident();
            let suffix = function.backend.many_iterator_suffix();
            let iterator = quote::format_ident!("{name}_{suffix}");
            let row_iter = &paths.row_iter;
            quote::quote! {
                pub #async_token fn #name #plain_lifetime(#client: #plain_client_ref impl #generic_client #arguments) -> Result<Vec<#row>, #error> {
                    #setup
                    let rows = #client.query(sql.as_str(), &values) #await_token?;
                    rows.iter().map(#row::from_row).collect()
                }
                pub #async_token fn #iterator #iterator_lifetime(#client: #iterator_client_ref impl #generic_client #arguments) -> Result<#row_iter, #error> {
                    #setup
                    #client.query_raw(sql.as_str(), values) #await_token
                }
            }
        }
        Annotation::Exec => quote::quote! {
            pub #async_token fn #name #plain_lifetime(#client: #plain_client_ref impl #generic_client #arguments) -> Result<(), #error> {
                #setup
                #client.execute(sql.as_str(), &values) #await_token.map(|_| ())
            }
        },
        Annotation::ExecRows | Annotation::ExecResult => quote::quote! {
            pub #async_token fn #name #plain_lifetime(#client: #plain_client_ref impl #generic_client #arguments) -> Result<u64, #error> {
                #setup
                #client.execute(sql.as_str(), &values) #await_token
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
    let lifetime = function
        .query
        .fields
        .iter()
        .any(|field| field.scalar_type().need_params_struct_lifetime())
        .then(|| syn::Lifetime::new("'p", proc_macro2::Span::call_site()));
    let generics = lifetime
        .as_ref()
        .map(|lifetime| quote::quote! {<#lifetime>});
    let params_ref = match &lifetime {
        Some(lifetime) => quote::quote! {&#lifetime #params},
        None => quote::quote! {&#params},
    };
    let values_ref = match &lifetime {
        Some(lifetime) => quote::quote! {&#lifetime},
        None => quote::quote! {&},
    };
    let dynamic = quote::format_ident!("{}_DYN", function.parts.constant);
    let args = params_common::dynamic_args(function.query);
    let binds = params_common::dynamic_binds(function.query)
        .into_iter()
        .map(|bind| {
            let pattern = bind.pattern;
            if bind.slice_element.is_some() {
                return quote::quote! {
                    #pattern => unreachable!("PostgreSQL binds sqlc slices as arrays"),
                };
            }
            let name = &bind.field.name;
            quote::quote! { #pattern => &params.#name as _, }
        });
    quote::quote! {
        fn #helper #generics(params: #params_ref) -> (String, Vec<#values_ref (dyn #to_sql + ::std::marker::Sync)>) {
            let args = [#(#args,)*];
            let (sql, binds) = #dynamic.build(&args);
            let values: Vec<&(dyn #to_sql + ::std::marker::Sync)> = binds
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

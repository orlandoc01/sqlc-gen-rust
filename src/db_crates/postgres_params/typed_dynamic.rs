use crate::query::Annotation;

use super::{Function, params_common, typed};

pub(super) fn functions(function: &Function<'_>) -> proc_macro2::TokenStream {
    let paths = &function.paths;
    let name = &function.name;
    let helper = quote::format_ident!("{name}_query");
    let client = &function.client;
    let arguments = &function.parts.arguments;
    let error = &paths.error;
    let async_token = &paths.async_token;
    let await_token = &paths.await_token;
    let plain_lifetime = &paths.plain.lifetime;
    let iterator_lifetime = &paths.iterator.lifetime;
    let plain_client_type = typed::client_type(function, &paths.plain);
    let iterator_client_type = typed::client_type(function, &paths.iterator);
    let helper_function = dynamic_helper(function, &helper);
    let setup = quote::quote! { let (sql, values) = #helper(&params); };
    let functions = match function.query.annotation {
        Annotation::One => {
            let row = function.row.struct_ident();
            let opt = quote::format_ident!("{name}_opt");
            quote::quote! {
                pub #async_token fn #name #plain_lifetime(#client: #plain_client_type #arguments) -> Result<#row, #error> {
                    #setup
                    let row = #client.query_typed_one(sql.as_str(), &values) #await_token?;
                    #row::from_row(&row)
                }
                pub #async_token fn #opt #plain_lifetime(#client: #plain_client_type #arguments) -> Result<Option<#row>, #error> {
                    #setup
                    #client.query_typed_opt(sql.as_str(), &values) #await_token?.map(|row| #row::from_row(&row)).transpose()
                }
            }
        }
        Annotation::Many => {
            let row = function.row.struct_ident();
            let suffix = function.backend.many_iterator_suffix();
            let iterator = quote::format_ident!("{name}_{suffix}");
            let row_iter = &paths.row_iter;
            quote::quote! {
                pub #async_token fn #name #plain_lifetime(#client: #plain_client_type #arguments) -> Result<Vec<#row>, #error> {
                    #setup
                    let rows = #client.query_typed(sql.as_str(), &values) #await_token?;
                    rows.iter().map(#row::from_row).collect()
                }
                pub #async_token fn #iterator #iterator_lifetime(#client: #iterator_client_type #arguments) -> Result<#row_iter, #error> {
                    #setup
                    #client.query_typed_raw(sql.as_str(), values.iter().map(|(value, typ)| (*value, typ.clone()))) #await_token
                }
            }
        }
        Annotation::Exec | Annotation::ExecRows | Annotation::ExecResult => {
            let returns_count = matches!(
                function.query.annotation,
                Annotation::ExecRows | Annotation::ExecResult
            );
            let raw = typed::execute_raw(
                function,
                quote::quote! {sql.as_str()},
                quote::quote! {values},
                returns_count,
            );
            typed::plain(
                function,
                &paths.plain,
                name,
                if returns_count {
                    quote::quote! {u64}
                } else {
                    quote::quote! {()}
                },
                quote::quote! { #setup #raw },
            )
        }
        _ => proc_macro2::TokenStream::new(),
    };
    quote::quote! { #helper_function #functions }
}

fn dynamic_helper(function: &Function<'_>, helper: &syn::Ident) -> proc_macro2::TokenStream {
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
    let to_sql = &function.paths.to_sql;
    let typ = &function.paths.typ;
    let dynamic_setup = params_common::dynamic_plan_setup(function.query, &function.parts.constant);
    let unknown_bind_arm = params_common::unknown_bind_arm();
    let binds = params_common::dynamic_binds(function.query).into_iter().map(|bind| {
        let pattern = bind.pattern;
        if bind.slice_element.is_some() {
            return quote::quote! { #pattern => unreachable!("PostgreSQL binds sqlc slices as arrays"), };
        }
        let name = &bind.field.name;
        let type_ident = super::super::postgres_types::type_ident(
            bind.field.scalar_type().db_type(),
            bind.field.scalar_type().array_dimensions(),
        )
        .expect("typed query has known PostgreSQL parameter types");
        quote::quote! { #pattern => (&params.#name as _, #typ::#type_ident), }
    });
    quote::quote! {
        fn #helper #generics(params: #params_ref) -> (String, Vec<(#values_ref (dyn #to_sql + ::std::marker::Sync), #typ)>) {
            #dynamic_setup
            let values = binds.iter().map(|bind| match bind {
                #(#binds)*
                #unknown_bind_arm
            }).collect();
            (sql, values)
        }
    }
}

use crate::query::Annotation;

use super::{Function, params_common, typed};

enum BindMode<'a> {
    Prepared,
    Typed(&'a [syn::Ident]),
}

impl BindMode<'_> {
    fn typed(&self) -> bool {
        matches!(self, Self::Typed(_))
    }

    fn values_type(
        &self,
        function: &Function<'_>,
        values_ref: &proc_macro2::TokenStream,
    ) -> proc_macro2::TokenStream {
        let to_sql = &function.paths.to_sql;
        let typ = &function.paths.typ;
        match self {
            Self::Prepared => quote::quote! {#values_ref (dyn #to_sql + ::std::marker::Sync)},
            Self::Typed(_) => {
                quote::quote! {(#values_ref (dyn #to_sql + ::std::marker::Sync), #typ)}
            }
        }
    }

    fn bind_arms(&self, function: &Function<'_>) -> proc_macro2::TokenStream {
        let typ = &function.paths.typ;
        params_common::dynamic_bind_arms(function.query, |bind| {
            let pattern = bind.pattern;
            if bind.slice_element.is_some() {
                return quote::quote! {
                    #pattern => unreachable!("PostgreSQL binds sqlc slices as arrays"),
                };
            }
            let name = &bind.field.name;
            match self {
                Self::Prepared => quote::quote! { #pattern => &params.#name as _, },
                Self::Typed(parameters) => {
                    let type_ident = &parameters[bind.index];
                    quote::quote! { #pattern => (&params.#name as _, #typ::#type_ident), }
                }
            }
        })
    }
}

pub(super) fn functions(function: &Function<'_>) -> proc_macro2::TokenStream {
    let typed_parameters = function.typed_parameters();
    let mode = typed_parameters
        .as_deref()
        .map(BindMode::Typed)
        .unwrap_or(BindMode::Prepared);
    let paths = &function.paths;
    let name = &function.name;
    let helper = quote::format_ident!("{name}_query");
    let client = &function.client;
    let await_token = &paths.await_token;
    let setup = quote::quote! { let (sql, values) = #helper(&params); };
    let helper_function = make_helper(function, &helper, &mode);
    let functions = match function.query.annotation {
        Annotation::One => {
            let row = function.row.struct_ident();
            let opt = quote::format_ident!("{name}_opt");
            let one_body = if mode.typed() {
                quote::quote! {
                    #setup
                    let row = #client.query_typed_one(sql.as_str(), &values) #await_token?;
                    #row::from_row(&row)
                }
            } else {
                quote::quote! {
                    #setup
                    let row = #client.query_one(sql.as_str(), &values) #await_token?;
                    #row::from_row(&row)
                }
            };
            let opt_body = if mode.typed() {
                quote::quote! { #setup #client.query_typed_opt(sql.as_str(), &values) #await_token?.map(|row| #row::from_row(&row)).transpose() }
            } else {
                quote::quote! { #setup #client.query_opt(sql.as_str(), &values) #await_token?.map(|row| #row::from_row(&row)).transpose() }
            };
            let one = function.plain_fn(
                &paths.plain,
                name,
                &quote::quote! {#row},
                mode.typed(),
                one_body,
            );
            let optional = function.plain_fn(
                &paths.plain,
                &opt,
                &quote::quote! {Option<#row>},
                mode.typed(),
                opt_body,
            );
            quote::quote! { #one #optional }
        }
        Annotation::Many => {
            let row = function.row.struct_ident();
            let suffix = function.backend.many_iterator_suffix();
            let iterator = quote::format_ident!("{name}_{suffix}");
            let row_iter = &paths.row_iter;
            let many_body = if mode.typed() {
                quote::quote! { #setup let rows = #client.query_typed(sql.as_str(), &values) #await_token?; rows.iter().map(#row::from_row).collect() }
            } else {
                quote::quote! { #setup let rows = #client.query(sql.as_str(), &values) #await_token?; rows.iter().map(#row::from_row).collect() }
            };
            let iterator_body = if mode.typed() {
                quote::quote! { #setup #client.query_typed_raw(sql.as_str(), values.iter().map(|(value, typ)| (*value, typ.clone()))) #await_token }
            } else {
                quote::quote! { #setup #client.query_raw(sql.as_str(), values) #await_token }
            };
            let many = function.plain_fn(
                &paths.plain,
                name,
                &quote::quote! {Vec<#row>},
                mode.typed(),
                many_body,
            );
            let iterator = function.plain_fn(
                &paths.iterator,
                &iterator,
                row_iter,
                mode.typed(),
                iterator_body,
            );
            quote::quote! { #many #iterator }
        }
        Annotation::Exec | Annotation::ExecRows | Annotation::ExecResult => {
            let returns_count = matches!(
                function.query.annotation,
                Annotation::ExecRows | Annotation::ExecResult
            );
            let return_type = if returns_count {
                quote::quote! {u64}
            } else {
                quote::quote! {()}
            };
            let body = if mode.typed() {
                let raw = typed::execute_raw(
                    function,
                    quote::quote! {sql.as_str()},
                    quote::quote! {values},
                    returns_count,
                );
                quote::quote! { #setup #raw }
            } else if returns_count {
                quote::quote! { #setup #client.execute(sql.as_str(), &values) #await_token }
            } else {
                quote::quote! { #setup #client.execute(sql.as_str(), &values) #await_token.map(|_| ()) }
            };
            function.plain_fn(&paths.plain, name, &return_type, mode.typed(), body)
        }
        _ => proc_macro2::TokenStream::new(),
    };
    quote::quote! { #helper_function #functions }
}

fn make_helper(
    function: &Function<'_>,
    helper: &syn::Ident,
    mode: &BindMode<'_>,
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
    let values_type = mode.values_type(function, &values_ref);
    let dynamic_setup = params_common::dynamic_plan_setup(function.query, &function.parts.constant);
    let binds = mode.bind_arms(function);
    quote::quote! {
        fn #helper #generics(params: #params_ref) -> (String, Vec<#values_type>) {
            #dynamic_setup
            let values = binds.iter().map(|bind| match bind { #binds }).collect();
            (sql, values)
        }
    }
}

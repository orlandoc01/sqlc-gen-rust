use crate::query::Annotation;

use super::Function;

pub(super) fn one_functions(function: &Function<'_>) -> proc_macro2::TokenStream {
    let parts = function.static_parts();
    let name = &function.name;
    let opt = quote::format_ident!("{name}_opt");
    let with = quote::format_ident!("{name}_with");
    let opt_with = quote::format_ident!("{name}_opt_with");
    let row = function.row.struct_ident();
    let values = values(function);
    let client = &function.client;
    let constant = &function.parts.constant;
    let await_token = &function.paths.await_token;
    let statement = &parts.statement;
    let values_ident = &parts.values_ident;
    let one = plain(
        function,
        &function.paths.plain,
        name,
        quote::quote! {#row},
        quote::quote! {
            #values
            let row = #client.query_typed_one(#constant, values) #await_token?;
            #row::from_row(&row)
        },
    );
    let optional = plain(
        function,
        &function.paths.plain,
        &opt,
        quote::quote! {Option<#row>},
        quote::quote! {
            #values
            #client.query_typed_opt(#constant, values) #await_token?.map(|row| #row::from_row(&row)).transpose()
        },
    );
    let one_with = function.with_fn(
        &function.paths.plain,
        &with,
        &quote::quote! {#row},
        &parts,
        quote::quote! {
            let row = #client.query_one(#statement, #values_ident) #await_token?;
            #row::from_row(&row)
        },
    );
    let optional_with = function.with_fn(
        &function.paths.plain,
        &opt_with,
        &quote::quote! {Option<#row>},
        &parts,
        quote::quote! {
            #client.query_opt(#statement, #values_ident) #await_token?.map(|row| #row::from_row(&row)).transpose()
        },
    );
    let prepare = function.prepare_function();
    quote::quote! { #prepare #one #one_with #optional #optional_with }
}

pub(super) fn many_functions(function: &Function<'_>) -> proc_macro2::TokenStream {
    let parts = function.static_parts();
    let name = &function.name;
    let with = quote::format_ident!("{name}_with");
    let suffix = function.backend.many_iterator_suffix();
    let iterator = quote::format_ident!("{name}_{suffix}");
    let iterator_with = quote::format_ident!("{name}_{suffix}_with");
    let row = function.row.struct_ident();
    let values = values(function);
    let client = &function.client;
    let constant = &function.parts.constant;
    let await_token = &function.paths.await_token;
    let statement = &parts.statement;
    let values_ident = &parts.values_ident;
    let many = plain(
        function,
        &function.paths.plain,
        name,
        quote::quote! {Vec<#row>},
        quote::quote! {
            #values
            let rows = #client.query_typed(#constant, values) #await_token?;
            rows.iter().map(#row::from_row).collect()
        },
    );
    let iterator_fn = plain(
        function,
        &function.paths.iterator,
        &iterator,
        function.paths.row_iter.clone(),
        quote::quote! {
            #values
            #client.query_typed_raw(#constant, values.iter().map(|(value, typ)| (*value, typ.clone()))) #await_token
        },
    );
    let many_with = function.with_fn(
        &function.paths.plain,
        &with,
        &quote::quote! {Vec<#row>},
        &parts,
        quote::quote! {
            let rows = #client.query(#statement, #values_ident) #await_token?;
            rows.iter().map(#row::from_row).collect()
        },
    );
    let iterator_with_fn = function.with_fn(
        &function.paths.iterator,
        &iterator_with,
        &function.paths.row_iter,
        &parts,
        quote::quote! {
            #client.query_raw(#statement, #values_ident.iter().copied()) #await_token
        },
    );
    let prepare = function.prepare_function();
    quote::quote! { #prepare #many #many_with #iterator_fn #iterator_with_fn }
}

pub(super) fn execute_functions(
    function: &Function<'_>,
    result: proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    let parts = function.static_parts();
    let name = &function.name;
    let with = quote::format_ident!("{name}_with");
    let returns_count = matches!(
        function.query.annotation,
        Annotation::ExecRows | Annotation::ExecResult
    );
    let execute = plain(
        function,
        &function.paths.plain,
        name,
        result.clone(),
        execute_body(function, returns_count),
    );
    let client = &function.client;
    let statement = &parts.statement;
    let values_ident = &parts.values_ident;
    let await_token = &function.paths.await_token;
    let with_body = match function.query.annotation {
        Annotation::Exec => {
            quote::quote! { #client.execute(#statement, #values_ident) #await_token.map(|_| ()) }
        }
        Annotation::ExecRows | Annotation::ExecResult => {
            quote::quote! { #client.execute(#statement, #values_ident) #await_token }
        }
        _ => unreachable!("only execute annotations reach typed execution"),
    };
    let execute_with = function.with_fn(&function.paths.plain, &with, &result, &parts, with_body);
    let prepare = function.prepare_function();
    quote::quote! { #prepare #execute #execute_with }
}

fn values(function: &Function<'_>) -> proc_macro2::TokenStream {
    let values = function.parts.local("values");
    let to_sql = &function.paths.to_sql;
    let typ = &function.paths.typ;
    let parameters = function.query.fields.iter().map(|field| {
        let value = function.parts.access.field(&field.name);
        let type_ident = super::super::postgres_types::type_ident(
            field.scalar_type().db_type(),
            field.scalar_type().array_dimensions(),
        )
        .expect("typed query has known PostgreSQL parameter types");
        quote::quote! { (&#value, #typ::#type_ident) }
    });
    quote::quote! {
        let #values: &[(&(dyn #to_sql + ::std::marker::Sync), #typ)] = &[#(#parameters,)*];
    }
}

pub(super) fn plain(
    function: &Function<'_>,
    signature: &super::super::postgres::ClientSignature,
    name: &syn::Ident,
    return_type: proc_macro2::TokenStream,
    body: proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    let client = &function.client;
    let arguments = &function.parts.arguments;
    let client_type = client_type(function, signature);
    let error = &function.paths.error;
    let async_token = &function.paths.async_token;
    let lifetime = &signature.lifetime;
    quote::quote! {
        pub #async_token fn #name #lifetime(#client: #client_type #arguments) -> Result<#return_type, #error> {
            #body
        }
    }
}

pub(super) fn client_type(
    function: &Function<'_>,
    signature: &super::super::postgres::ClientSignature,
) -> proc_macro2::TokenStream {
    let client_ref = &signature.client_ref;
    match function.backend {
        super::super::postgres::Postgres::Deadpool => {
            quote::quote! {#client_ref deadpool_postgres::Client}
        }
        super::super::postgres::Postgres::Sync | super::super::postgres::Postgres::Tokio => {
            let generic_client = &function.paths.client;
            quote::quote! {#client_ref impl #generic_client}
        }
    }
}

fn execute_body(function: &Function<'_>, returns_count: bool) -> proc_macro2::TokenStream {
    let sql = &function.parts.constant;
    let values_ident = function.parts.local("values");
    let values_definition = values(function);
    let raw = execute_raw(
        function,
        quote::quote! {#sql},
        quote::quote! {#values_ident},
        returns_count,
    );
    quote::quote! { #values_definition #raw }
}

pub(super) fn execute_raw(
    function: &Function<'_>,
    sql: proc_macro2::TokenStream,
    values: proc_macro2::TokenStream,
    returns_count: bool,
) -> proc_macro2::TokenStream {
    let client = &function.client;
    let result = if returns_count {
        quote::quote! {Ok(rows_affected)}
    } else {
        quote::quote! {Ok(())}
    };
    let rows_affected = returns_count.then(|| {
        quote::quote! {let rows_affected = it.rows_affected().unwrap_or(0);}
    });
    let stream_rows_affected = returns_count.then(|| {
        quote::quote! {let rows_affected = s.rows_affected().unwrap_or(0);}
    });
    match function.backend {
        super::super::postgres::Postgres::Sync => quote::quote! {
            let mut it = #client.query_typed_raw(#sql, #values.iter().map(|(value, typ)| (*value, typ.clone())))?;
            while postgres::fallible_iterator::FallibleIterator::next(&mut it)?.is_some() {}
            #rows_affected
            #result
        },
        super::super::postgres::Postgres::Tokio | super::super::postgres::Postgres::Deadpool => {
            quote::quote! {
                let mut s = ::std::pin::pin!(#client.query_typed_raw(#sql, #values.iter().map(|(value, typ)| (*value, typ.clone()))).await?);
                while futures_util::TryStreamExt::try_next(&mut s).await?.is_some() {}
                #stream_rows_affected
                #result
            }
        }
    }
}

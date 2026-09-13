use crate::db_crates::postgres::{ClientSignature, Postgres};

use super::Function;

pub(super) fn values(
    function: &Function<'_>,
    values: &syn::Ident,
    parameters: &[syn::Ident],
) -> proc_macro2::TokenStream {
    let to_sql = &function.paths.to_sql;
    let typ = &function.paths.typ;
    let parameters = function
        .query
        .fields
        .iter()
        .zip(parameters)
        .map(|(field, type_ident)| {
            let value = function.parts.access.field(&field.name);
            quote::quote! { (&#value, #typ::#type_ident) }
        });
    quote::quote! {
        let #values: &[(&(dyn #to_sql + ::std::marker::Sync), #typ)] = &[#(#parameters,)*];
    }
}

pub(super) fn client_type(
    function: &Function<'_>,
    signature: &ClientSignature,
) -> proc_macro2::TokenStream {
    let client_ref = &signature.client_ref;
    match function.backend {
        Postgres::Deadpool => {
            quote::quote! {#client_ref deadpool_postgres::Client}
        }
        Postgres::Sync | Postgres::Tokio => {
            let generic_client = &function.paths.client;
            quote::quote! {#client_ref impl #generic_client}
        }
    }
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
        quote::quote! {
            let rows_affected = rows.rows_affected().unwrap_or(0);
        }
    });
    match function.backend {
        Postgres::Sync => quote::quote! {
            let mut rows = #client.query_typed_raw(#sql, #values.iter().map(|(value, typ)| (*value, typ.clone())))?;
            while postgres::fallible_iterator::FallibleIterator::next(&mut rows)?.is_some() {}
            #rows_affected
            #result
        },
        Postgres::Tokio | Postgres::Deadpool => {
            quote::quote! {
                let mut rows = ::std::pin::pin!(#client.query_typed_raw(#sql, #values.iter().map(|(value, typ)| (*value, typ.clone()))).await?);
                while futures_util::TryStreamExt::try_next(&mut rows).await?.is_some() {}
                #rows_affected
                #result
            }
        }
    }
}

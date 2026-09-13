use convert_case::{Case, Casing as _};

use crate::{query::Query, value_ident};

#[derive(Clone, Copy)]
pub(crate) enum ParameterAccess {
    Direct,
    Struct,
}

impl ParameterAccess {
    pub(crate) fn field(self, name: &syn::Ident) -> proc_macro2::TokenStream {
        match self {
            Self::Direct => quote::quote! {#name},
            Self::Struct => quote::quote! {params.#name},
        }
    }
}

pub(crate) struct QueryParts {
    pub(crate) constant: syn::Ident,
    pub(crate) params: proc_macro2::TokenStream,
    pub(crate) arguments: proc_macro2::TokenStream,
    pub(crate) access: ParameterAccess,
    taken: Vec<String>,
}

impl QueryParts {
    pub(crate) fn local(&self, base: &str) -> syn::Ident {
        let mut name = base.to_string();
        while self.taken.contains(&name) {
            name.push('_');
        }
        quote::format_ident!("{name}")
    }
}

pub(crate) fn query_parts(query: &Query, query_parameter_limit: usize) -> QueryParts {
    let (access, taken) = if uses_params_struct(query, query_parameter_limit) {
        (ParameterAccess::Struct, Vec::new())
    } else {
        let names = query.fields.iter().map(|field| field.name.to_string());
        (ParameterAccess::Direct, names.collect())
    };
    QueryParts {
        constant: query_const_ident(query),
        params: params_definition(query, query_parameter_limit),
        arguments: function_arguments(query, query_parameter_limit),
        access,
        taken,
    }
}

pub(crate) fn query_const_ident(query: &Query) -> syn::Ident {
    query_ident(&query.query_name, Case::UpperSnake)
}

pub(crate) fn query_function_ident(query: &Query) -> syn::Ident {
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

fn params_definition(query: &Query, query_parameter_limit: usize) -> proc_macro2::TokenStream {
    if !uses_params_struct(query, query_parameter_limit) {
        return proc_macro2::TokenStream::new();
    }

    let params = params_ident(query);
    let lifetime = syn::Lifetime::new("'a", proc_macro2::Span::call_site());
    let derive = if query
        .fields
        .iter()
        .all(|field| field.scalar_type().can_default())
    {
        quote::quote! {#[derive(Debug, Clone, Default)]}
    } else {
        quote::quote! {#[derive(Debug, Clone)]}
    };
    let fields = query.fields.iter().map(|field| {
        let name = &field.name;
        let typ = field.scalar_type().to_params_struct_tokens(Some(&lifetime));
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
            #derive
            pub struct #params<#lifetime> {
                #(#fields,)*
                #(#flags,)*
            }
        }
    } else {
        quote::quote! {
            #derive
            pub struct #params {
                #(#fields,)*
                #(#flags,)*
            }
        }
    }
}

fn function_arguments(query: &Query, query_parameter_limit: usize) -> proc_macro2::TokenStream {
    if uses_params_struct(query, query_parameter_limit) {
        let params = params_type(query);
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

pub(crate) fn params_type(query: &Query) -> proc_macro2::TokenStream {
    let params = params_ident(query);
    if query
        .fields
        .iter()
        .any(|field| field.scalar_type().need_params_struct_lifetime())
    {
        quote::quote! {#params<'_>}
    } else {
        quote::quote! {#params}
    }
}

fn params_ident(query: &Query) -> syn::Ident {
    value_ident(&format!("{}Params", query.query_name))
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

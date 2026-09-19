use convert_case::{Case, Casing as _};

use crate::{
    dynfilter::{
        DynFilterInfo, Switch,
        variants::{Expansion, Variant},
    },
    query::Query,
    value_ident,
};

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

pub(crate) struct QueryParts<'a> {
    pub(crate) constant: syn::Ident,
    pub(crate) params: proc_macro2::TokenStream,
    pub(crate) arguments: proc_macro2::TokenStream,
    pub(crate) access: ParameterAccess,
    /// Present only for a cache-eligible query under `dynfilters.prepared`.
    pub(crate) prepared: Option<PreparedParts<'a>>,
    taken: Vec<String>,
}

pub(crate) struct PreparedParts<'a> {
    /// `X_VARIANTS`: every runtime-exact SQL text of the query.
    pub(crate) variants_ident: syn::Ident,
    /// `prepare_x`: the per-query warm-up on statement-caching backends.
    pub(crate) warmup_ident: syn::Ident,
    pub(crate) variants: &'a [Variant],
}

impl<'a> QueryParts<'a> {
    pub(crate) fn local(&self, base: &str) -> syn::Ident {
        let mut name = base.to_string();
        while self.taken.contains(&name) {
            name.push('_');
        }
        quote::format_ident!("{name}")
    }
}

pub(crate) fn query_parts<'a>(
    query: &Query,
    query_parameter_limit: usize,
    prepared: Option<&'a Expansion<'_>>,
) -> QueryParts<'a> {
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
        prepared: prepared.map(|prepared| PreparedParts {
            variants_ident: variants_const_ident(query),
            warmup_ident: warmup_ident(query),
            variants: &prepared.variants,
        }),
        taken,
    }
}

pub(crate) fn variants_const_ident(query: &Query) -> syn::Ident {
    quote::format_ident!("{}_VARIANTS", query_const_ident(query))
}

pub(crate) fn warmup_ident(query: &Query) -> syn::Ident {
    quote::format_ident!("prepare_{}", query_function_ident(query))
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
    let flags = query
        .dynfilter()
        .into_iter()
        .flat_map(|info| control_fields(query, info));
    let enums = query.dynfilter().into_iter().flat_map(|info| {
        info.switches
            .iter()
            .map(|switch| switch_enum(query, switch))
    });

    let generics = query
        .params_need_lifetime()
        .then(|| quote::quote! {<#lifetime>});
    quote::quote! {
        #(#enums)*
        #derive
        pub struct #params #generics {
            #(#fields,)*
            #(#flags,)*
        }
    }
}

/// `(annotation name, field ident, type)` for every bool flag and switch enum field, in
/// bind-slot order.
pub(crate) fn control_field_types<'a>(
    query: &'a Query,
    info: &'a DynFilterInfo,
) -> impl Iterator<Item = (&'a str, syn::Ident, proc_macro2::TokenStream)> + 'a {
    info.flag_params
        .iter()
        .enumerate()
        .filter_map(move |(index, flag)| match flag.switch {
            None => Some((
                flag.name.as_str(),
                crate::field_ident(&flag.name),
                quote::quote! {bool},
            )),
            // A switch's choices occupy contiguous slots, so its first slot is the one whose
            // predecessor belongs to something else.
            Some(switch) if index == 0 || info.flag_params[index - 1].switch != Some(switch) => {
                let switch = &info.switches[switch];
                let ident = switch_enum_ident(query, switch);
                Some((
                    switch.field.as_str(),
                    crate::field_ident(&switch.field),
                    quote::quote! {#ident},
                ))
            }
            Some(_) => None,
        })
}

fn control_fields<'a>(
    query: &'a Query,
    info: &'a DynFilterInfo,
) -> impl Iterator<Item = proc_macro2::TokenStream> + 'a {
    control_field_types(query, info).map(|(_, name, typ)| quote::quote! {pub #name: #typ})
}

fn switch_enum(query: &Query, switch: &Switch) -> proc_macro2::TokenStream {
    let ident = switch_enum_ident(query, switch);
    let variants = switch.choices.iter().map(|choice| {
        let variant = switch_variant_ident(choice);
        let default = (*choice == switch.default).then(|| quote::quote! {#[default]});
        quote::quote! {#default #variant}
    });
    quote::quote! {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
        pub enum #ident {
            #(#variants,)*
        }
    }
}

pub(crate) fn switch_enum_ident(query: &Query, switch: &Switch) -> syn::Ident {
    value_ident(&format!("{}_{}", query.query_name, switch.field))
}

pub(crate) fn switch_variant_ident(choice: &str) -> syn::Ident {
    value_ident(choice)
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
    if query.params_need_lifetime() {
        quote::quote! {#params<'_>}
    } else {
        quote::quote! {#params}
    }
}

pub(crate) fn params_struct_ident(
    query: &Query,
    query_parameter_limit: usize,
) -> Option<syn::Ident> {
    uses_params_struct(query, query_parameter_limit).then(|| params_ident(query))
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

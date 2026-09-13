use crate::query::{Annotation, Query, QueryError, ReturningRows};

pub(crate) use super::params_common_types::{
    ParameterAccess, QueryParts, params_type, query_function_ident,
};
use super::params_common_types::{query_const_ident, query_parts};

/// One entry of the dynamic bind plan `match`: which `dynfilter::Bind` pattern it answers and
/// how the backend reaches the value. Ownership decisions stay with the backend.
pub(crate) struct DynamicBind<'a> {
    pub(crate) index: usize,
    pub(crate) pattern: proc_macro2::TokenStream,
    pub(crate) field: &'a crate::query::ColumnField,
    pub(crate) conditional: bool,
    /// `&params.x[element]` for slice parameters, already unwrapped when conditional.
    pub(crate) slice_element: Option<proc_macro2::TokenStream>,
}

pub(crate) fn dynamic_binds(query: &Query) -> Vec<DynamicBind<'_>> {
    let info = query.dynfilter().expect("dynamic query");
    query
        .fields
        .iter()
        .enumerate()
        .map(|(index, field)| {
            let name = &field.name;
            let number = query.param_number(index);
            let arg_index = number - 1;
            let conditional = info.conditional_param_numbers.contains(&number);
            if query.is_sqlc_slice(index) {
                let element = if conditional {
                    quote::quote! {&params.#name.unwrap()[element]}
                } else {
                    quote::quote! {&params.#name[element]}
                };
                DynamicBind {
                    index,
                    pattern: quote::quote! {dynfilter::Bind::Elem(#arg_index, element)},
                    field,
                    conditional,
                    slice_element: Some(element),
                }
            } else {
                DynamicBind {
                    index,
                    pattern: quote::quote! {dynfilter::Bind::Arg(#arg_index)},
                    field,
                    conditional,
                    slice_element: None,
                }
            }
        })
        .collect()
}

pub(crate) trait ParamsGenerator {
    fn placeholders(&self) -> proc_macro2::TokenStream;
    fn returning_row(&self, row: &ReturningRows) -> proc_macro2::TokenStream;
    fn generated_functions(&self, query: &Query) -> Vec<GeneratedFunction>;
    fn query_functions(
        &self,
        query: &Query,
        row: &ReturningRows,
        parts: &QueryParts,
    ) -> proc_macro2::TokenStream;
}

pub(crate) struct GeneratedFunction {
    pub(crate) ident: syn::Ident,
    pub(crate) helper: &'static str,
}

pub(crate) fn simple_generated_functions(
    query: &Query,
    supported: impl FnOnce(Annotation) -> bool,
) -> Vec<GeneratedFunction> {
    let name = query_function_ident(query);
    match query.annotation {
        Annotation::One => vec![
            GeneratedFunction {
                ident: name.clone(),
                helper: "query function",
            },
            GeneratedFunction {
                ident: quote::format_ident!("{name}_opt"),
                helper: "optional query helper",
            },
        ],
        annotation if supported(annotation) => vec![GeneratedFunction {
            ident: name,
            helper: "query function",
        }],
        _ => Vec::new(),
    }
}

pub(crate) fn generate_queries<G: ParamsGenerator>(
    generator: &G,
    rows: &[ReturningRows],
    queries: &[Query],
    query_parameter_limit: usize,
) -> Result<proc_macro2::TokenStream, QueryError> {
    validate_generated_items(generator, queries)?;
    let query_tokens = rows
        .iter()
        .zip(queries)
        .map(|(row, query)| generate_query(generator, row, query, query_parameter_limit));
    let dynfilter_runtime = queries
        .iter()
        .any(|query| query.dynfilter().is_some())
        .then(dynfilter_runtime)
        .unwrap_or_default();
    let query_index = queries.iter().map(|query| {
        let name = syn::LitStr::new(&query.query_name, proc_macro2::Span::call_site());
        let constant = query_const_ident(query);
        quote::quote! {(#name, #constant)}
    });

    Ok(quote::quote! {
        #dynfilter_runtime
        #(#query_tokens)*
        pub const QUERIES: &[(&str, &str)] = &[
            #(#query_index,)*
        ];
    })
}

fn validate_generated_items<G: ParamsGenerator>(
    generator: &G,
    queries: &[Query],
) -> Result<(), QueryError> {
    let mut items = std::collections::BTreeMap::new();
    register_generated_item(
        &mut items,
        "QUERIES",
        GeneratedFunction {
            ident: quote::format_ident!("QUERIES"),
            helper: "query index",
        },
    )?;
    for query in queries {
        for GeneratedFunction { ident, helper } in generator.generated_functions(query) {
            register_generated_item(
                &mut items,
                &query.query_name,
                GeneratedFunction { ident, helper },
            )?;
        }
        register_generated_item(
            &mut items,
            &query.query_name,
            GeneratedFunction {
                ident: query_const_ident(query),
                helper: "SQL constant",
            },
        )?;
        if query.dynfilter().is_some() {
            let constant = query_const_ident(query);
            register_generated_item(
                &mut items,
                &query.query_name,
                GeneratedFunction {
                    ident: quote::format_ident!("{constant}_DYN"),
                    helper: "dynamic plan",
                },
            )?;
        }
    }
    Ok(())
}

fn register_generated_item(
    items: &mut std::collections::BTreeMap<String, (String, &'static str)>,
    query_name: &str,
    GeneratedFunction { ident, helper }: GeneratedFunction,
) -> Result<(), QueryError> {
    let ident = ident.to_string();
    if let Some((first_query_name, first_helper)) =
        items.insert(ident.clone(), (query_name.to_string(), helper))
    {
        return Err(QueryError::conflicting_generated_function(
            first_query_name,
            first_helper,
            query_name.to_string(),
            helper,
            ident,
        ));
    }
    Ok(())
}

fn generate_query<G: ParamsGenerator>(
    generator: &G,
    row: &ReturningRows,
    query: &Query,
    query_parameter_limit: usize,
) -> proc_macro2::TokenStream {
    let parts = query_parts(query, query_parameter_limit);
    let sql = query.query_str();
    let dynamic = dynamic_static(query, &parts.constant, generator.placeholders());
    let returns = matches!(query.annotation, Annotation::One | Annotation::Many)
        .then(|| generator.returning_row(row));
    let functions = generator.query_functions(query, row, &parts);
    let constant = &parts.constant;
    let params = &parts.params;

    quote::quote! {
        pub const #constant: &str = #sql;
        #dynamic
        #params
        #returns
        #functions
    }
}

pub(crate) fn dynamic_static(
    query: &Query,
    constant: &syn::Ident,
    placeholders: proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    if query.dynfilter().is_none() {
        return proc_macro2::TokenStream::new();
    }
    let dynamic = quote::format_ident!("{constant}_DYN");
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

pub(crate) fn dynamic_args(query: &Query) -> Vec<proc_macro2::TokenStream> {
    let info = query.dynfilter().expect("dynamic query");
    let mut fields = query.fields.iter().enumerate().collect::<Vec<_>>();
    fields.sort_unstable_by_key(|(index, _)| query.param_number(*index));
    let fields = fields.into_iter().map(|(index, field)| {
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
    });
    let flags = info.flag_params.iter().map(|flag| {
        let name = crate::field_ident(&flag.name);
        quote::quote! {dynfilter::Arg::Flag(params.#name)}
    });
    fields.chain(flags).collect()
}

pub(crate) fn dynamic_plan_setup(query: &Query, constant: &syn::Ident) -> proc_macro2::TokenStream {
    let dynamic = quote::format_ident!("{constant}_DYN");
    let args = dynamic_args(query);
    quote::quote! {
        let args = [#(#args,)*];
        let (sql, binds) = #dynamic.build(&args);
    }
}

pub(crate) fn unknown_bind_arm() -> proc_macro2::TokenStream {
    quote::quote! {
        _ => unreachable!("dynfilter bind plan referenced an unknown argument"),
    }
}

pub(crate) fn dynamic_bind_arms(
    query: &Query,
    mut arm: impl FnMut(DynamicBind<'_>) -> proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    let arms = dynamic_binds(query).into_iter().map(&mut arm);
    let unknown = unknown_bind_arm();
    quote::quote! { #(#arms)* #unknown }
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

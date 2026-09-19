use crate::query::{Annotation, Query, QueryError, ReturningRows};

pub(crate) use super::params_common_types::{
    ParameterAccess, PreparedParts, QueryParts, params_type, query_function_ident,
};
use super::params_common_types::{
    control_field_types, params_struct_ident, query_const_ident, query_parts, switch_enum_ident,
    switch_variant_ident, variants_const_ident, warmup_ident,
};
use crate::dynfilter::{prepared::Prepared, variants::Expansion};

mod items;
use items::validate_generated_items;

pub(crate) struct GenerateOptions<'a> {
    pub(crate) query_parameter_limit: usize,
    /// `None` when `dynfilters.prepared` is off. An empty map is a package without expanded
    /// dynamic queries, which still emits the empty aggregate.
    pub(crate) prepared: Option<&'a Prepared<'a>>,
}

impl Default for GenerateOptions<'_> {
    fn default() -> Self {
        Self {
            query_parameter_limit: 1,
            prepared: None,
        }
    }
}

impl<'a> GenerateOptions<'a> {
    fn prepared_for(&self, query: &Query) -> Option<&'a Expansion<'a>> {
        self.prepared
            .and_then(|prepared| prepared.0.get(query.query_name.as_str()))
            .filter(|expansion| expansion.cached)
    }
}

pub(crate) const AGGREGATE_WARMUP: &str = "prepare_dynfilter_variants";
pub(crate) const VARIANT_COUNT: &str = "DYNFILTER_VARIANT_COUNT";

/// One entry of the dynamic bind plan `match`: which `dynfilter::Bind` pattern it answers and
/// how the backend reaches the value. Ownership decisions stay with the backend.
pub(crate) struct DynamicBind<'a> {
    pub(crate) pattern: proc_macro2::TokenStream,
    pub(crate) field: &'a crate::query::ColumnField,
    pub(crate) conditional: bool,
    /// `&params.x[element]` for slice parameters, already unwrapped when conditional.
    pub(crate) slice_element: Option<proc_macro2::TokenStream>,
}

pub(crate) fn dynamic_binds(query: &Query) -> Vec<DynamicBind<'_>> {
    let info = query.expect_dynfilter();
    query
        .fields
        .iter()
        .enumerate()
        .map(|(index, field)| {
            let name = &field.name;
            let number = query.param_number(index);
            let arg_index = number - 1;
            let conditional = info.conditional_param_numbers.contains(&number);
            let (pattern, slice_element) = if query.is_sqlc_slice(index) {
                let element = if conditional {
                    quote::quote! {&params.#name.unwrap()[element]}
                } else {
                    quote::quote! {&params.#name[element]}
                };
                (
                    quote::quote! {dynfilter::Bind::Elem(#arg_index, element)},
                    Some(element),
                )
            } else {
                (quote::quote! {dynfilter::Bind::Arg(#arg_index)}, None)
            };
            DynamicBind {
                pattern,
                field,
                conditional,
                slice_element,
            }
        })
        .collect()
}

pub(crate) trait ParamsGenerator {
    /// The placeholder form the generated code renders at run time; the same value drives the
    /// emitted `compile_with_arg_order` call and the plugin-side rendering of prepared texts.
    fn placeholders(&self) -> crate::dynfilter_runtime::Placeholders;
    fn returning_row(&self, row: &ReturningRows) -> proc_macro2::TokenStream;
    fn generated_functions(&self, query: &Query) -> Vec<GeneratedItem>;
    fn query_functions(
        &self,
        query: &Query,
        row: &ReturningRows,
        parts: &QueryParts<'_>,
    ) -> proc_macro2::TokenStream;
    /// Whether the backend keeps a per-connection statement cache the generated code can use.
    /// Backends without one still emit `X_VARIANTS` constants but no warm-up, aggregate, or
    /// count.
    fn caches_statements(&self) -> bool;
    /// Rejects prepared metadata the backend cannot warm safely; SQLx PostgreSQL checks that
    /// one SQL text never needs two bind-type vectors.
    fn validate_prepared(
        &self,
        _queries: &[Query],
        _prepared: &Prepared<'_>,
    ) -> Result<(), QueryError> {
        Ok(())
    }
    /// The `prepare_x` warm-up for one cache-eligible query.
    fn prepare_functions(
        &self,
        query: &Query,
        parts: &PreparedParts<'_>,
    ) -> proc_macro2::TokenStream;
    /// The `prepare_dynfilter_variants` aggregate calling each warm-up in file order.
    fn prepare_all(&self, functions: &[syn::Ident]) -> proc_macro2::TokenStream;
}

pub(crate) struct GeneratedItem {
    pub(crate) ident: syn::Ident,
    pub(crate) helper: &'static str,
}

pub(crate) fn simple_generated_functions(
    query: &Query,
    supported: impl FnOnce(Annotation) -> bool,
) -> Vec<GeneratedItem> {
    let name = query_function_ident(query);
    match query.annotation {
        Annotation::One => vec![
            GeneratedItem {
                ident: name.clone(),
                helper: "query function",
            },
            GeneratedItem {
                ident: quote::format_ident!("{name}_opt"),
                helper: "optional query helper",
            },
        ],
        annotation if supported(annotation) => vec![GeneratedItem {
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
    options: &GenerateOptions<'_>,
) -> Result<proc_macro2::TokenStream, QueryError> {
    validate_generated_items(generator, rows, queries, options)?;
    if let Some(prepared) = options.prepared {
        generator.validate_prepared(queries, prepared)?;
    }
    let query_tokens = rows
        .iter()
        .zip(queries)
        .map(|(row, query)| generate_query(generator, row, query, options));
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
    let warmups = prepared_warmups(generator, queries, options);

    Ok(quote::quote! {
        #dynfilter_runtime
        #(#query_tokens)*
        pub const QUERIES: &[(&str, &str)] = &[
            #(#query_index,)*
        ];
        #warmups
    })
}

/// The aggregate warm-up and variant count, emitted (even empty) whenever the flag is on and
/// the backend caches statements. Cache-eligible queries are listed in file order.
fn prepared_warmups<G: ParamsGenerator>(
    generator: &G,
    queries: &[Query],
    options: &GenerateOptions<'_>,
) -> proc_macro2::TokenStream {
    if options.prepared.is_none() || !generator.caches_statements() {
        return proc_macro2::TokenStream::new();
    }
    let eligible = queries
        .iter()
        .filter_map(|query| {
            options
                .prepared_for(query)
                .map(|prepared| (query, prepared))
        })
        .collect::<Vec<_>>();
    let functions = eligible
        .iter()
        .map(|(query, _)| warmup_ident(query))
        .collect::<Vec<_>>();
    let count = eligible
        .iter()
        .map(|(_, prepared)| prepared.variants.len())
        .sum::<usize>();
    let aggregate = generator.prepare_all(&functions);
    let count = proc_macro2::Literal::usize_unsuffixed(count);
    let count_ident = quote::format_ident!("{VARIANT_COUNT}");
    quote::quote! {
        #aggregate
        pub const #count_ident: usize = #count;
    }
}

fn generate_query<G: ParamsGenerator>(
    generator: &G,
    row: &ReturningRows,
    query: &Query,
    options: &GenerateOptions<'_>,
) -> proc_macro2::TokenStream {
    let parts = query_parts(
        query,
        options.query_parameter_limit,
        options.prepared_for(query),
    );
    let sql = query.query_str();
    let dynamic = dynamic_static(
        query,
        &parts.constant,
        placeholders_tokens(generator.placeholders()),
    );
    let variants = parts.prepared.as_ref().map(|prepared| {
        let ident = &prepared.variants_ident;
        let texts = prepared
            .variants
            .iter()
            .map(|variant| crate::query::raw_string_literal(&variant.sql));
        quote::quote! { pub const #ident: &[&str] = &[#(#texts,)*]; }
    });
    let warmup = parts
        .prepared
        .as_ref()
        .filter(|_| generator.caches_statements())
        .map(|prepared| generator.prepare_functions(query, prepared));
    let returns = matches!(query.annotation, Annotation::One | Annotation::Many)
        .then(|| generator.returning_row(row));
    let functions = generator.query_functions(query, row, &parts);
    let constant = &parts.constant;
    let params = &parts.params;

    quote::quote! {
        pub const #constant: &str = #sql;
        #variants
        #dynamic
        #params
        #returns
        #functions
        #warmup
    }
}

/// The runtime enum value spelled as the generated module's path to it.
fn placeholders_tokens(
    placeholders: crate::dynfilter_runtime::Placeholders,
) -> proc_macro2::TokenStream {
    use crate::dynfilter_runtime::Placeholders;
    match placeholders {
        Placeholders::Numbered => quote::quote! {dynfilter::Placeholders::Numbered},
        Placeholders::NumberedSqlite => quote::quote! {dynfilter::Placeholders::NumberedSqlite},
        Placeholders::Question => quote::quote! {dynfilter::Placeholders::Question},
    }
}

pub(crate) fn dynamic_static(
    query: &Query,
    constant: &syn::Ident,
    placeholders: proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    let Some(info) = query.dynfilter() else {
        return proc_macro2::TokenStream::new();
    };
    let dynamic = quote::format_ident!("{constant}_DYN");
    let arg_order = query.arg_order();
    let plan = &info.plan;
    quote::quote! {
        static #dynamic: std::sync::LazyLock<dynfilter::Compiled> =
            std::sync::LazyLock::new(|| {
                dynfilter::compile_with_arg_order(
                    #constant,
                    #placeholders,
                    &[#(#arg_order,)*],
                    &[#(#plan,)*],
                )
            });
    }
}

pub(crate) fn dynamic_args(query: &Query) -> Vec<proc_macro2::TokenStream> {
    let info = query.expect_dynfilter();
    let slots = query.arg_slots();
    let fields = slots.params.iter().map(|slot| {
        let name = &query.fields[slot.field_index].name;
        match (slot.slice, slot.conditional) {
            (true, true) => quote::quote! {dynfilter::Arg::Slice(params.#name.map(<[_]>::len))},
            (true, false) => quote::quote! {dynfilter::Arg::Slice(Some(params.#name.len()))},
            (false, true) => quote::quote! {dynfilter::Arg::from_option(&params.#name)},
            (false, false) => quote::quote! {dynfilter::Arg::Active},
        }
    });
    let flags = info.flag_params.iter().map(|flag| match flag.switch {
        Some(switch) => {
            let switch = &info.switches[switch];
            let field = crate::field_ident(&switch.field);
            let enum_ident = switch_enum_ident(query, switch);
            let variant = switch_variant_ident(&flag.name);
            quote::quote! {dynfilter::Arg::Flag(matches!(params.#field, #enum_ident::#variant))}
        }
        None => {
            let name = crate::field_ident(&flag.name);
            quote::quote! {dynfilter::Arg::Flag(params.#name)}
        }
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

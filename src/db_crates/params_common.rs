use crate::query::{Annotation, Query, QueryError, ReturningRows};

pub(crate) use super::params_common_types::{
    ParameterAccess, PreparedParts, QueryParts, params_type, query_function_ident,
};
use super::params_common_types::{
    control_field_types, params_struct_ident, query_const_ident, query_parts, states_const_ident,
    switch_enum_ident, switch_variant_ident, variants_const_ident, warmup_ident,
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
    /// The engine lexing rules the runtime compiles with; independent of `placeholders`.
    fn dialect(&self) -> crate::dynfilter_runtime::Dialect;
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
    /// How a warm-up reaches the connection; only called when `caches_statements`.
    fn warmup(&self) -> Warmup;
    /// The body of one cache-eligible query's `prepare_x` warm-up, ending before `Ok(())`.
    fn warmup_body(&self, query: &Query, parts: &PreparedParts<'_>) -> proc_macro2::TokenStream;
}

/// The backend-specific half of a warm-up signature: `pub [async] fn f(<param>: <client>) ->
/// <result>`, where `result` is the backend's `Result<(), Error>` spelling.
pub(crate) struct Warmup {
    /// The connection parameter's name, which `warmup_body` and `prepare_each` bodies use.
    pub(crate) param: &'static str,
    pub(crate) client: proc_macro2::TokenStream,
    pub(crate) result: proc_macro2::TokenStream,
    pub(crate) asynchronous: bool,
}

impl Warmup {
    fn function(
        &self,
        name: &syn::Ident,
        param: &syn::Ident,
        body: proc_macro2::TokenStream,
    ) -> proc_macro2::TokenStream {
        let Self {
            client,
            result,
            asynchronous,
            ..
        } = self;
        let asynchronous = asynchronous.then(|| quote::quote! {async});
        quote::quote! {
            pub #asynchronous fn #name(#param: #client) -> #result {
                #body
                Ok(())
            }
        }
    }

    /// The connection parameter as the body spells it.
    pub(crate) fn param_ident(&self) -> syn::Ident {
        quote::format_ident!("{}", self.param)
    }

    /// `prepare_x` for one query.
    fn prepare(
        &self,
        parts: &PreparedParts<'_>,
        body: proc_macro2::TokenStream,
    ) -> proc_macro2::TokenStream {
        self.function(&parts.warmup_ident, &self.param_ident(), body)
    }

    /// `prepare_dynfilter_variants`, calling each warm-up in file order.
    fn aggregate(&self, functions: &[syn::Ident]) -> proc_macro2::TokenStream {
        let underscore = if functions.is_empty() { "_" } else { "" };
        let client = quote::format_ident!("{underscore}{}", self.param);
        let awaited = self.asynchronous.then(|| quote::quote! {.await});
        self.function(
            &quote::format_ident!("{AGGREGATE_WARMUP}"),
            &client,
            quote::quote! { #(self::#functions(#client) #awaited?;)* },
        )
    }

    /// `for sql in X_VARIANTS { <prepare>?; }`, where `prepare` receives the connection
    /// parameter and has `sql` in scope.
    pub(crate) fn prepare_each(
        &self,
        parts: &PreparedParts<'_>,
        prepare: impl FnOnce(&syn::Ident) -> proc_macro2::TokenStream,
    ) -> proc_macro2::TokenStream {
        let variants = &parts.variants_ident;
        let prepare = prepare(&self.param_ident());
        let awaited = self.asynchronous.then(|| quote::quote! {.await});
        quote::quote! {
            for sql in #variants {
                #prepare #awaited?;
            }
        }
    }
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
    let aggregate = generator.warmup().aggregate(&functions);
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
        dialect_tokens(generator.dialect()),
    );
    let variants = parts.prepared.as_ref().map(|prepared| {
        let ident = &prepared.variants_ident;
        let texts = prepared
            .variants
            .iter()
            .map(|variant| crate::query::raw_string_literal(&variant.sql));
        let states = prepared.states.map(|states| {
            let states_ident = &prepared.states_ident;
            let entries = states.iter().map(|state| {
                let gates = proc_macro2::Literal::u128_unsuffixed(state.gates);
                let variant = proc_macro2::Literal::usize_unsuffixed(state.variant);
                let binds = state.binds.iter().map(bind_tokens);
                quote::quote! {
                    dynfilter::State { gates: #gates, sql: #ident[#variant], binds: &[#(#binds,)*] }
                }
            });
            quote::quote! { pub const #states_ident: &[dynfilter::State] = &[#(#entries,)*]; }
        });
        quote::quote! {
            pub const #ident: &[&str] = &[#(#texts,)*];
            #states
        }
    });
    let warmup = parts
        .prepared
        .as_ref()
        .filter(|_| generator.caches_statements())
        .map(|prepared| {
            generator
                .warmup()
                .prepare(prepared, generator.warmup_body(query, prepared))
        });
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

/// The runtime enum value spelled as the generated module's path to it.
fn dialect_tokens(dialect: crate::dynfilter_runtime::Dialect) -> proc_macro2::TokenStream {
    use crate::dynfilter_runtime::Dialect;
    match dialect {
        Dialect::Postgres => quote::quote! {dynfilter::Dialect::Postgres},
        Dialect::MySql => quote::quote! {dynfilter::Dialect::MySql},
        Dialect::Sqlite => quote::quote! {dynfilter::Dialect::Sqlite},
    }
}

pub(crate) fn dynamic_static(
    query: &Query,
    constant: &syn::Ident,
    placeholders: proc_macro2::TokenStream,
    dialect: proc_macro2::TokenStream,
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
                    #dialect,
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

fn bind_tokens(bind: &crate::dynfilter_runtime::Bind) -> proc_macro2::TokenStream {
    use crate::dynfilter_runtime::Bind;
    match bind {
        Bind::Arg(arg) => quote::quote! {dynfilter::Bind::Arg(#arg)},
        Bind::Elem(arg, element) => quote::quote! {dynfilter::Bind::Elem(#arg, #element)},
    }
}

/// Binds `sql` and `binds` for the call: from the `X_STATES` table when one was emitted
/// (`&'static str` and `&'static [Bind]`), otherwise rendered per call (`String` and `Vec`).
pub(crate) fn dynamic_plan_setup(
    query: &Query,
    parts: &QueryParts<'_>,
) -> proc_macro2::TokenStream {
    let constant = &parts.constant;
    let dynamic = quote::format_ident!("{constant}_DYN");
    let args = dynamic_args(query);
    let plan = match parts.states_ident() {
        Some(states) => quote::quote! {
            let state = #dynamic
                .state(#states, &args)
                .expect("every control state of a prepared dynamic query is enumerated");
            let (sql, binds) = (state.sql, state.binds);
        },
        None => quote::quote! { let (sql, binds) = #dynamic.build(&args); },
    };
    quote::quote! {
        let args = [#(#args,)*];
        #plan
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

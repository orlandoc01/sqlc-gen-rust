//! The registry of every top-level item the generated module emits, so two queries (or a query
//! and a shared helper) can never claim one Rust name silently.

use super::{
    AGGREGATE_WARMUP, GenerateOptions, GeneratedItem, ParamsGenerator, VARIANT_COUNT,
    control_field_types, params_struct_ident, query_const_ident, states_const_ident,
    switch_enum_ident, switch_variant_ident, variants_const_ident, warmup_ident,
};
use crate::query::{Annotation, Query, QueryError, ReturningRows};
use crate::unique::insert_unique;

/// Every top-level item the module emits for `query` on this backend and pass, so the
/// registry only ever reserves names that are actually generated.
fn emitted_items<G: ParamsGenerator>(
    generator: &G,
    row: &ReturningRows,
    query: &Query,
    options: &GenerateOptions<'_>,
    embedded: &mut std::collections::BTreeSet<String>,
) -> Vec<GeneratedItem> {
    let item = |ident: syn::Ident, helper: &'static str| GeneratedItem { ident, helper };
    let mut items = Vec::new();
    if let Some(prepared) = options.prepared_for(query) {
        items.push(item(variants_const_ident(query), "variant SQL constant"));
        if prepared.states.is_some() {
            items.push(item(states_const_ident(query), "variant state table"));
        }
        if generator.caches_statements() {
            items.push(item(warmup_ident(query), "prepared-statement warm-up"));
        }
    }
    if matches!(query.annotation, Annotation::One | Annotation::Many) {
        items.push(item(row.struct_ident(), "row struct"));
    }
    items.extend(
        row.embedded_tables()
            .filter(|table| embedded.insert(table.qualified_name.clone()))
            .map(|table| item(table.ident.clone(), "embedded table struct")),
    );
    items.extend(generator.generated_functions(query));
    items.push(item(query_const_ident(query), "SQL constant"));
    items.extend(
        params_struct_ident(query, options.query_parameter_limit)
            .map(|ident| item(ident, "params struct")),
    );
    if let Some(info) = query.dynfilter() {
        let constant = query_const_ident(query);
        items.push(item(quote::format_ident!("{constant}_DYN"), "dynamic plan"));
        items.extend(
            info.switches
                .iter()
                .map(|switch| item(switch_enum_ident(query, switch), "switch enum")),
        );
    }
    items
}

pub(super) fn validate_generated_items<G: ParamsGenerator>(
    generator: &G,
    rows: &[ReturningRows],
    queries: &[Query],
    options: &GenerateOptions<'_>,
) -> Result<(), QueryError> {
    let mut items = std::collections::BTreeMap::new();
    let mut embedded = std::collections::BTreeSet::new();
    let module_items = [("QUERIES", "query index")].into_iter().chain(
        (options.prepared.is_some() && generator.caches_statements())
            .then_some([
                (AGGREGATE_WARMUP, "prepared-statement warm-up"),
                (VARIANT_COUNT, "variant count"),
            ])
            .into_iter()
            .flatten(),
    );
    for (ident, helper) in module_items {
        register_generated_item(
            &mut items,
            "the module",
            quote::format_ident!("{ident}"),
            helper,
        )?;
    }
    for (row, query) in rows.iter().zip(queries) {
        for GeneratedItem { ident, helper } in
            emitted_items(generator, row, query, options, &mut embedded)
        {
            register_generated_item(&mut items, &query.query_name, ident, helper)?;
        }
        let Some(info) = query.dynfilter() else {
            continue;
        };
        for switch in &info.switches {
            validate_unique(
                query,
                "switch variant",
                switch
                    .choices
                    .iter()
                    .map(|choice| (switch_variant_ident(choice).to_string(), choice.as_str())),
            )?;
        }
        validate_unique(
            query,
            "params struct field",
            query
                .fields
                .iter()
                .map(crate::query::ColumnField::conflict_pair)
                .chain(
                    control_field_types(query, info)
                        .map(|(source, ident, _)| (ident.to_string(), source.to_string())),
                ),
        )?;
    }
    Ok(())
}

fn validate_unique(
    query: &Query,
    item: &'static str,
    idents: impl Iterator<Item = (String, impl std::fmt::Display)>,
) -> Result<(), QueryError> {
    crate::query::validate_unique_members(
        format_args!("Query `{}`", query.query_name),
        item,
        idents.map(|(ident, source)| (ident, source.to_string())),
    )
}

fn register_generated_item(
    items: &mut std::collections::BTreeMap<String, (String, &'static str)>,
    query_name: &str,
    ident: syn::Ident,
    helper: &'static str,
) -> Result<(), QueryError> {
    let ident = ident.to_string();
    insert_unique(items, ident.clone(), (query_name.to_string(), helper)).map_err(
        |(first_query_name, first_helper)| {
            QueryError::conflicting_generated_function(
                first_query_name,
                first_helper,
                query_name.to_string(),
                helper,
                ident,
            )
        },
    )
}

use std::collections::HashMap;

use super::DynFilterInfo;
use crate::dynfilter_runtime::{Arg, Bind, Compiled, Placeholders, compile_with_arg_order};
use crate::query::{ArgSlots, Query, QueryError};

/// Raw control states enumerated per query before dedup; past this, even a query whose
/// variants dedup below the limit would take too long to render.
pub(crate) const MAX_RAW_STATES: usize = 1 << 20;

#[derive(Debug)]
pub(crate) struct Variant {
    pub(crate) sql: String,
    pub(crate) binds: Vec<Bind>,
    /// The first bind plan of another control state that renders the same text but binds
    /// different type classes; only collected when `Options::bind_classes` is set. One prepared
    /// statement serves every state of a text, so two classes for one text is a conflict.
    pub(crate) alternate: Option<Vec<Bind>>,
}

/// Maps each runtime argument index to a backend-defined bind-type class, so states can be
/// compared by a short signature instead of by retained bind plans.
pub(crate) type BindClasses<'a> = &'a dyn Fn(&Query) -> Vec<usize>;

pub(crate) struct Expansion<'q> {
    pub(crate) query: &'q Query,
    pub(crate) variants: Vec<Variant>,
    /// Emits constants and warm-ups and takes the cached call path. A cache-skipped or
    /// slice-expanding query keeps its variants only for the shared-key bind-type check, since
    /// it still reads whatever the connection cache holds for a text.
    pub(crate) cached: bool,
}

pub(crate) struct Options<'a> {
    pub(crate) placeholders: Placeholders,
    pub(crate) limit: usize,
    pub(crate) skip: &'a [String],
    /// `dynfilters.prepared_skip`: expanded, never cached.
    pub(crate) cache_skip: &'a [String],
    /// Set only when a backend prepares by text and needs type-conflict detection.
    pub(crate) bind_classes: Option<BindClasses<'a>>,
}

/// Every name in a skip list must be a query with dynamic-filter controls; `option` names the
/// list in the error.
pub(crate) fn validate_skip_names(
    queries: &[Query],
    names: &[String],
    option: &str,
) -> Result<(), QueryError> {
    match names.iter().find(|name| {
        !queries
            .iter()
            .any(|query| query.query_name == **name && query.has_dynamic_controls())
    }) {
        Some(name) => Err(QueryError::dynamic_filter(
            name.clone(),
            format!("{option} names no dynamic-filter query"),
        )),
        None => Ok(()),
    }
}

/// Every base query with dynamic-filter controls, in request order, minus the skip list.
pub(crate) fn expand_all<'q>(
    queries: &'q [Query],
    options: &Options<'_>,
) -> Result<Vec<Expansion<'q>>, QueryError> {
    validate_skip_names(queries, options.skip, "dynfilters.variants_skip")?;
    queries
        .iter()
        .filter(|query| query.has_dynamic_controls() && !options.skip.contains(&query.query_name))
        .map(|query| {
            let classes = options.bind_classes.map(|classes| classes(query));
            Ok(Expansion {
                query,
                variants: expand(
                    query,
                    options.placeholders,
                    options.limit,
                    classes.as_deref(),
                )?,
                cached: !options.cache_skip.contains(&query.query_name)
                    && !query.arg_slots().params.iter().any(|slot| slot.slice),
            })
        })
        .collect()
}

fn compile(query: &Query, placeholders: Placeholders) -> Compiled {
    let info = query.expect_dynfilter();
    compile_with_arg_order(
        &info.annotated_sql,
        placeholders,
        &query.arg_order(),
        &info.runtime_plan(),
    )
}

fn skip_hint(query: &Query, message: String, remedy: &str) -> QueryError {
    QueryError::dynamic_filter(
        query.query_name.clone(),
        format!("{message}; {remedy} or add the query to dynfilters.variants_skip"),
    )
}

pub(crate) fn expand(
    query: &Query,
    placeholders: Placeholders,
    limit: usize,
    bind_classes: Option<&[usize]>,
) -> Result<Vec<Variant>, QueryError> {
    let info = query.expect_dynfilter();
    let compiled = compile(query, placeholders);
    let slots = query.arg_slots();
    let controls = controls(info, &slots);
    let total = controls
        .iter()
        .try_fold(1usize, |total, control| total.checked_mul(control.radix))
        .filter(|total| *total <= MAX_RAW_STATES)
        .ok_or_else(|| {
            skip_hint(
                query,
                format!("dynamic-filter control states exceed the {MAX_RAW_STATES} raw-state cap"),
                "reduce its independent controls",
            )
        })?;
    // Every control rewrites all of its own slots on every state, so one args vector serves
    // the whole enumeration.
    let mut args = base_args(info, &slots);
    let class_of = |bind: &Bind| {
        let (Bind::Arg(arg) | Bind::Elem(arg, _)) = bind;
        bind_classes.map(|classes| classes[*arg])
    };
    let mut seen = HashMap::<String, usize>::new();
    // One class signature per distinct text, kept only when a classifier is supplied.
    let mut signatures = Vec::<Vec<usize>>::new();
    (0..total).try_fold(Vec::new(), |mut variants: Vec<Variant>, counter| {
        controls.iter().rev().fold(counter, |rest, control| {
            control.apply(&mut args, rest % control.radix);
            rest / control.radix
        });
        let (sql, binds) = compiled.build(&args);
        if let Some(index) = seen.get(&sql) {
            let variant = &mut variants[*index];
            if variant.alternate.is_none()
                && bind_classes.is_some()
                && !binds
                    .iter()
                    .filter_map(class_of)
                    .eq(signatures[*index].iter().copied())
            {
                variant.alternate = Some(binds);
            }
        } else {
            seen.insert(sql.clone(), variants.len());
            if bind_classes.is_some() {
                signatures.push(binds.iter().filter_map(class_of).collect());
            }
            variants.push(Variant {
                sql,
                binds,
                alternate: None,
            });
        }
        if variants.len() > limit {
            return Err(skip_hint(
                query,
                format!("dynamic-filter variants exceed dynfilters.variant_limit {limit}"),
                "raise dynfilters.variant_limit",
            ));
        }
        Ok(variants)
    })
}

/// Every parameter present (slices with one element) and every flag off.
fn base_args(info: &DynFilterInfo, slots: &ArgSlots) -> Vec<Arg> {
    slots
        .params
        .iter()
        .map(|slot| {
            if slot.slice {
                Arg::Slice(Some(1))
            } else {
                Arg::Active
            }
        })
        .chain(info.flag_params.iter().map(|_| Arg::Flag(false)))
        .collect()
}

struct Control {
    radix: usize,
    kind: Kind,
}

enum Kind {
    Param { slot: usize, slice: bool },
    Flag { slot: usize },
    Switch { slots: Vec<usize> },
}

impl Control {
    fn apply(&self, args: &mut [Arg], value: usize) {
        match &self.kind {
            Kind::Param { slot, slice } => {
                args[*slot] = match (value == 1, slice) {
                    (true, true) => Arg::Slice(Some(1)),
                    (false, true) => Arg::Slice(None),
                    (true, false) => Arg::Active,
                    (false, false) => Arg::Inactive,
                }
            }
            Kind::Flag { slot } => args[*slot] = Arg::Flag(value == 1),
            Kind::Switch { slots } => {
                for (choice, slot) in slots.iter().enumerate() {
                    args[*slot] = Arg::Flag(choice == value);
                }
            }
        }
    }
}

/// Conditional parameters, then independent flags, then switches, so a counter over them
/// yields a stable variant order.
fn controls(info: &DynFilterInfo, slots: &ArgSlots) -> Vec<Control> {
    let params = slots
        .params
        .iter()
        .enumerate()
        .filter(|(_, param)| param.conditional)
        .map(|(slot, param)| Control {
            radix: 2,
            kind: Kind::Param {
                slot,
                slice: param.slice,
            },
        });
    let flags = info
        .flag_params
        .iter()
        .enumerate()
        .filter(|(_, flag)| flag.switch.is_none())
        .map(|(offset, _)| Control {
            radix: 2,
            kind: Kind::Flag {
                slot: slots.flag_slot(offset),
            },
        });
    // `references()` allocates one flag slot per switch choice, but `Control::apply` indexes
    // `slots` by choice position, so resolve each slot by choice name rather than by claim order.
    let mut slots_by_switch = vec![Vec::new(); info.switches.len()];
    for (offset, flag) in info.flag_params.iter().enumerate() {
        if let Some(switch) = flag.switch {
            slots_by_switch[switch].push((flag.name.as_str(), slots.flag_slot(offset)));
        }
    }
    let switches = info
        .switches
        .iter()
        .zip(slots_by_switch)
        .map(|(switch, named_slots)| {
            let slots = switch
                .choices
                .iter()
                .map(|choice| {
                    named_slots
                        .iter()
                        .find(|(name, _)| name == choice)
                        .map(|(_, slot)| *slot)
                        .expect("every switch choice claims a flag slot")
                })
                .collect();
            Control {
                radix: switch.choices.len(),
                kind: Kind::Switch { slots },
            }
        });
    params.chain(flags).chain(switches).collect()
}

//! Warm-ups for the SQLx statement cache. PostgreSQL prepares with the Rust bind types the
//! generated code will use, because sqlx keys its cache by SQL text and an untyped `prepare`
//! lets the server infer types that a later typed bind may not match.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use super::Sqlx;
use crate::db_crates::params_common::{ParamsGenerator as _, PreparedParts};
use crate::dynfilter::prepared::Prepared;
use crate::dynfilter_runtime::Bind;
use crate::query::{Query, QueryError};

pub(super) fn warmup_body(
    sqlx: &Sqlx,
    query: &Query,
    parts: &PreparedParts<'_>,
) -> proc_macro2::TokenStream {
    let variants = &parts.variants_ident;
    let conn = sqlx.warmup().param_ident();
    match sqlx {
        Sqlx::Postgres => {
            // sqlx 0.8 names every persistent statement on the server but only tracks it in
            // an enabled cache, so with `statement_cache_capacity(0)` each prepare leaks. The
            // first prepare either lands in the cache or proves it is disabled; stop there.
            let guard = quote::quote! {
                if sqlx::Connection::cached_statements_size(&*#conn) == 0 {
                    return Err(sqlx::Error::Configuration(
                        "dynfilters.prepared warm-up needs statement_cache_capacity > 0: sqlx 0.8 leaks named PostgreSQL statements when the cache is disabled".into(),
                    ));
                }
            };
            let bound = BoundTypes::new(query);
            let calls = parts.variants.iter().enumerate().map(|(index, variant)| {
                let guard = (index == 0).then_some(&guard);
                let index = proc_macro2::Literal::usize_unsuffixed(index);
                let types = bound.types(&variant.binds);
                quote::quote! {
                    sqlx::Executor::prepare_with(&mut *#conn, #variants[#index], &[#(#types,)*]).await?;
                    #guard
                }
            });
            quote::quote! { #(#calls)* }
        }
        Sqlx::MySql | Sqlx::Sqlite => sqlx.warmup().prepare_each(parts, |conn| {
            quote::quote! { sqlx::Executor::prepare(&mut *#conn, sql) }
        }),
    }
}

/// `<T as sqlx::Type<sqlx::Postgres>>::type_info()` per runtime argument index, resolved once
/// per query from the type the generated `.bind()` passes (overrides and nullability included,
/// minus the `Option` a conditional parameter unwraps). Slice elements never reach a warm-up:
/// expanding-slice queries are not cache-eligible.
struct BoundTypes {
    by_arg: Vec<proc_macro2::TokenStream>,
}

impl BoundTypes {
    fn new(query: &Query) -> Self {
        // `arg_slots().params` is already in argument order.
        let by_arg = query
            .arg_slots()
            .params
            .iter()
            .map(|slot| {
                let typ = query.fields[slot.field_index]
                    .scalar_type()
                    .to_bound_tokens(slot.conditional);
                type_info(typ)
            })
            .collect();
        Self { by_arg }
    }

    fn types(&self, binds: &[Bind]) -> Vec<proc_macro2::TokenStream> {
        binds
            .iter()
            .map(|bind| {
                let (Bind::Arg(arg) | Bind::Elem(arg, _)) = bind;
                self.by_arg[*arg].clone()
            })
            .collect()
    }
}

fn type_info(typ: proc_macro2::TokenStream) -> proc_macro2::TokenStream {
    quote::quote! { <#typ as sqlx::Type<sqlx::Postgres>>::type_info() }
}

/// The type expression one cache entry is keyed on. sqlx reports one `type_info` for `T` and
/// `Option<T>`, so the `Option` a nullable column adds is dropped before comparing, exactly
/// as a conditional parameter's is.
fn cache_type(field: &crate::query::ColumnField) -> String {
    type_info(field.scalar_type().to_bound_tokens(true)).to_string()
}

/// Numbers each argument's `cache_type` so the variant enumerator can compare control states
/// by a short class signature, with the same notion of sameness `validate_bind_types` uses.
pub(crate) fn bind_classes(query: &Query) -> Vec<usize> {
    let mut interner = Interner::default();
    query
        .arg_slots()
        .params
        .iter()
        .map(|slot| interner.intern(cache_type(&query.fields[slot.field_index])))
        .collect()
}

/// Type expressions stringified once per argument and compared by id across queries.
#[derive(Default)]
struct Interner {
    ids: HashMap<String, usize>,
    names: Vec<String>,
}

impl Interner {
    fn intern(&mut self, name: String) -> usize {
        *self.ids.entry(name).or_insert_with_key(|name| {
            self.names.push(name.clone());
            self.names.len() - 1
        })
    }

    fn render(&self, ids: &[usize]) -> String {
        ids.iter()
            .map(|id| self.names[*id].as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// One SQL text that reaches the connection cache: a static query, or one control state of a
/// dynamic query.
struct Source<'a> {
    query: &'a str,
    kind: SourceKind,
    /// Interned `cache_type` per bind, in bind order.
    types: Vec<usize>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SourceKind {
    Static,
    Dynamic { cached: bool },
}

impl SourceKind {
    /// Inserts into the cache: static queries and cache-eligible dynamic queries. A skipped
    /// dynamic query only reads the cache, but a hit binds its values to whatever was prepared.
    fn producer(self) -> bool {
        match self {
            Self::Static => true,
            Self::Dynamic { cached } => cached,
        }
    }
}

impl std::fmt::Display for SourceKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Static => "static query",
            Self::Dynamic { cached: true } => "cached dynamic query",
            Self::Dynamic { cached: false } => "cache-skipped dynamic query",
        })
    }
}

/// sqlx keys its per-connection cache by SQL text and consults it before honouring
/// `persistent(false)`, so every source of one text must bind the same Rust types whenever any
/// source still inserts into the cache. The plugin cannot evaluate consumer `Type` impls and
/// requires identical emitted type expressions. Queries in other generated modules, hand-written
/// SQL, and expansion-skipped dynamic queries are outside this check.
pub(super) fn validate_bind_types(
    queries: &[Query],
    prepared: &Prepared<'_>,
) -> Result<(), QueryError> {
    let mut by_sql: BTreeMap<&str, Vec<Source<'_>>> = BTreeMap::new();
    let mut interner = Interner::default();
    for query in queries {
        match prepared.0.get(query.query_name.as_str()) {
            None if query.dynfilter().is_none() => {
                by_sql.entry(query.sql()).or_default().push(Source {
                    query: &query.query_name,
                    kind: SourceKind::Static,
                    types: query
                        .fields
                        .iter()
                        .map(|field| interner.intern(cache_type(field)))
                        .collect(),
                });
            }
            None => {}
            Some(metadata) => {
                let by_arg = query
                    .arg_slots()
                    .params
                    .iter()
                    .map(|slot| interner.intern(cache_type(&query.fields[slot.field_index])))
                    .collect::<Vec<_>>();
                for variant in &metadata.variants {
                    for binds in std::iter::once(&variant.binds).chain(&variant.alternate) {
                        by_sql.entry(&variant.sql).or_default().push(Source {
                            query: &query.query_name,
                            kind: SourceKind::Dynamic {
                                cached: metadata.cached,
                            },
                            types: binds
                                .iter()
                                .map(|bind| {
                                    let (Bind::Arg(arg) | Bind::Elem(arg, _)) = bind;
                                    by_arg[*arg]
                                })
                                .collect(),
                        });
                    }
                }
            }
        }
    }
    for (sql, sources) in &by_sql {
        let first = &sources[0];
        let Some(conflict) = sources.iter().find(|source| source.types != first.types) else {
            continue;
        };
        if !sources.iter().any(|source| source.kind.producer()) {
            continue;
        }
        let names = sources
            .iter()
            .map(|source| source.query)
            .collect::<BTreeSet<_>>();
        let statics = sources
            .iter()
            .filter(|source| source.kind == SourceKind::Static)
            .map(|source| format!("`{}`", source.query))
            .collect::<BTreeSet<_>>();
        let fix = if statics.is_empty() {
            format!(
                "add every dynamic query sharing the text to dynfilters.prepared_skip: {}",
                names
                    .iter()
                    .map(|name| format!("`{name}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        } else {
            format!(
                "a static query always caches its text, so change the SQL of {} or of the dynamic query",
                statics.into_iter().collect::<Vec<_>>().join(", ")
            )
        };
        return Err(QueryError::dynamic_filter(
            first.query.to_string(),
            format!(
                "SQL text `{}` is bound as [{}] by {} `{}` but as [{}] by {} `{}`; one connection cache entry would serve both, so {fix}",
                sql.replace('\n', " "),
                interner.render(&first.types),
                first.kind,
                first.query,
                interner.render(&conflict.types),
                conflict.kind,
                conflict.query,
            ),
        ));
    }
    Ok(())
}

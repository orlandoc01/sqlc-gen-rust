//! Warm-ups for the SQLx statement cache. PostgreSQL prepares with the Rust bind types the
//! generated code will use, because sqlx keys its cache by SQL text and an untyped `prepare`
//! lets the server infer types that a later typed bind may not match.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use super::Sqlx;
use crate::db_crates::params_common::{AGGREGATE_WARMUP, PreparedParts};
use crate::dynfilter::prepared::Prepared;
use crate::dynfilter_runtime::Bind;
use crate::query::{Query, QueryError};

pub(super) fn warmup(
    sqlx: &Sqlx,
    query: &Query,
    parts: &PreparedParts<'_>,
) -> proc_macro2::TokenStream {
    let name = &parts.warmup_ident;
    let variants = &parts.variants_ident;
    let connection = sqlx.connection_ident();
    let body = match sqlx {
        Sqlx::Postgres => {
            // sqlx 0.8 names every persistent statement on the server but only tracks it in
            // an enabled cache, so with `statement_cache_capacity(0)` each prepare leaks. The
            // first prepare either lands in the cache or proves it is disabled; stop there.
            let guard = quote::quote! {
                if sqlx::Connection::cached_statements_size(&*conn) == 0 {
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
                    sqlx::Executor::prepare_with(&mut *conn, #variants[#index], &[#(#types,)*]).await?;
                    #guard
                }
            });
            quote::quote! { #(#calls)* }
        }
        Sqlx::MySql | Sqlx::Sqlite => quote::quote! {
            for sql in #variants {
                sqlx::Executor::prepare(&mut *conn, sql).await?;
            }
        },
    };
    quote::quote! {
        pub async fn #name(conn: &mut #connection) -> Result<(), sqlx::Error> {
            #body
            Ok(())
        }
    }
}

pub(super) fn aggregate(sqlx: &Sqlx, functions: &[syn::Ident]) -> proc_macro2::TokenStream {
    let name = quote::format_ident!("{AGGREGATE_WARMUP}");
    let connection = sqlx.connection_ident();
    let conn = if functions.is_empty() {
        quote::format_ident!("_conn")
    } else {
        quote::format_ident!("conn")
    };
    quote::quote! {
        pub async fn #name(#conn: &mut #connection) -> Result<(), sqlx::Error> {
            #(self::#functions(#conn).await?;)*
            Ok(())
        }
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
                quote::quote! { <#typ as sqlx::Type<sqlx::Postgres>>::type_info() }
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

    fn signature(&self, binds: &[Bind]) -> String {
        self.types(binds)
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Numbers each argument's bound type so the variant enumerator can compare control states by
/// a short class signature; equal classes mean identical emitted type expressions.
pub(crate) fn bind_classes(query: &Query) -> Vec<usize> {
    let mut classes = HashMap::<String, usize>::new();
    BoundTypes::new(query)
        .by_arg
        .iter()
        .map(|typ| {
            let next = classes.len();
            *classes.entry(typ.to_string()).or_insert(next)
        })
        .collect()
}

/// One SQL text that reaches the connection cache: a static query, or one control state of a
/// dynamic query.
struct Source<'a> {
    query: &'a str,
    /// Inserts into the cache: static queries and cache-eligible dynamic queries. A skipped
    /// dynamic query only reads the cache, but a hit binds its values to whatever was prepared.
    producer: bool,
    kind: SourceKind,
    types: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SourceKind {
    Static,
    Dynamic { cached: bool },
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
    for query in queries {
        match prepared.0.get(query.query_name.as_str()) {
            None if query.dynfilter().is_none() => {
                let types = query
                    .fields
                    .iter()
                    .map(|field| {
                        let typ = field.scalar_type().to_bound_tokens(false);
                        quote::quote! { <#typ as sqlx::Type<sqlx::Postgres>>::type_info() }
                            .to_string()
                    })
                    .collect::<Vec<_>>();
                by_sql.entry(query.sql()).or_default().push(Source {
                    query: &query.query_name,
                    producer: true,
                    kind: SourceKind::Static,
                    types: types.join(", "),
                });
            }
            None => {}
            Some(metadata) => {
                let bound = BoundTypes::new(query);
                for variant in &metadata.variants {
                    for binds in std::iter::once(&variant.binds).chain(&variant.alternate) {
                        by_sql.entry(&variant.sql).or_default().push(Source {
                            query: &query.query_name,
                            producer: metadata.cached,
                            kind: SourceKind::Dynamic {
                                cached: metadata.cached,
                            },
                            types: bound.signature(binds),
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
        if !sources.iter().any(|source| source.producer) {
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
                first.types,
                first.kind,
                first.query,
                conflict.types,
                conflict.kind,
                conflict.query,
            ),
        ));
    }
    Ok(())
}

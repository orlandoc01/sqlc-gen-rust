//! Runtime-exact variant texts for `dynfilters.prepared`, enumerated in memory from the base
//! queries. A package without eligible queries holds an empty map.

use std::collections::HashMap;

use super::DynFilters;
use super::variants::{Expansion, Options, expand_all};
use crate::db_crates::DbCrate;
use crate::query::{Query, QueryError};

/// Every expanded dynamic query by name; `Expansion::cached` marks the ones that emit prepared
/// artifacts and take the cached call path.
pub(crate) struct Prepared<'q>(pub(crate) HashMap<&'q str, Expansion<'q>>);

impl<'q> Prepared<'q> {
    pub(crate) fn enumerate(
        queries: &'q [Query],
        db_crate: DbCrate,
        options: &DynFilters,
    ) -> Result<Self, QueryError> {
        let expansions = expand_all(
            queries,
            &Options {
                placeholders: db_crate.runtime_placeholders(),
                limit: options.variant_limit,
                skip: &options.variants_skip,
                cache_skip: &options.prepared_skip,
                bind_classes: db_crate.bind_classes(),
            },
        )?;
        Ok(Self(
            expansions
                .into_iter()
                .map(|expansion| (expansion.query.query_name.as_str(), expansion))
                .collect(),
        ))
    }
}

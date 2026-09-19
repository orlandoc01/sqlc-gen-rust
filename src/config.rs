use crate::query::{DbTypeMap, Query, ReturnRowAttributes, RsType};
use crate::{Error, db_crates};

#[derive(Debug, Clone, serde::Deserialize, Default)]
#[serde(default)]
pub(crate) struct OverrideType {
    /// Override db type
    db_type: Option<String>,
    /// Override column name
    column: Option<String>,
    /// Override Rust type
    rs_type: String,
    /// Rust type's slice if have
    rs_slice: Option<String>,
    /// Marker is copy cheap
    copy_cheap: bool,
    /// Marker the type implements `Default`, so params structs using it can derive it
    can_default: bool,
}

#[derive(Debug, serde::Deserialize)]
#[serde(default)]
pub(crate) struct Config {
    pub(crate) output: String,
    pub(crate) db_crate: db_crates::DbCrate,
    pub(crate) api: Option<String>,
    pub(crate) query_parameter_limit: usize,
    pub(crate) overrides: Vec<OverrideType>,
    pub(crate) debug: bool,
    #[serde(flatten)]
    pub(crate) return_row_attributes: ReturnRowAttributes,
    pub(crate) enum_derives: Vec<String>,
    pub(crate) dynfilters: crate::dynfilter::DynFilters,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            output: "queries.rs".into(),
            db_crate: Default::default(),
            api: None,
            query_parameter_limit: 1,
            overrides: Default::default(),
            debug: false,
            return_row_attributes: Default::default(),
            enum_derives: Vec::new(),
            dynfilters: Default::default(),
        }
    }
}

impl Config {
    pub(crate) fn from_option(buf: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(buf)
    }

    pub(crate) fn validate(&self, queries: &[Query]) -> Result<(), Error> {
        if self.api.as_deref() == Some("builder") {
            return Err(Error::any(
                "the builder API was removed; only params_struct is generated".into(),
            ));
        }

        if let Some(query) = queries
            .iter()
            .find(|query| !self.db_crate.supports(query.annotation))
        {
            return Err(Error::any(
                format!(
                    "params_struct does not support {} with {} ({}).",
                    query.annotation, self.db_crate, query.query_name
                )
                .into(),
            ));
        }

        self.dynfilters.validate(queries)
    }
}

pub(crate) fn apply_overrides(config: &Config, db_type: &mut DbTypeMap) -> Result<(), Error> {
    for override_type in &config.overrides {
        let owned_type = syn::parse_str::<syn::Type>(&override_type.rs_type)
            .map_err(|e| Error::any(e.into()))?;
        let slice_type = override_type
            .rs_slice
            .as_deref()
            .map(syn::parse_str::<syn::Type>)
            .transpose()
            .map_err(|e| Error::any(e.into()))?;

        let rs_type = RsType::new(owned_type, slice_type, override_type.copy_cheap)
            .with_can_default(override_type.can_default);
        match (
            override_type.db_type.as_deref(),
            override_type.column.as_deref(),
        ) {
            (None, Some(column)) => db_type.insert_column_type(column, rs_type),
            (Some(db_type_name), None) => db_type.insert_db_type(db_type_name, rs_type),
            (Some(_), Some(_)) => {
                let message = "Cannot override both db_type and column name at the same time.";
                return Err(Error::any(message.into()));
            }
            (None, None) => {
                let message = "Must override either db_type or column name.";
                return Err(Error::any(message.into()));
            }
        }
    }
    Ok(())
}

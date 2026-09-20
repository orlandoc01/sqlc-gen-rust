use quote::ToTokens;
use regex_lite::Regex;
use sqlparser::{
    dialect::{Dialect as SqlDialect, MySqlDialect, PostgreSqlDialect, SQLiteDialect},
    tokenizer::{Token, Whitespace},
};
use std::collections::HashMap;
use std::sync::LazyLock;

mod attachment;
mod directive;
pub(crate) mod prepared;
mod render;
pub(crate) mod resolve;
pub(crate) mod slots;
mod source;
pub(crate) use directive::{Directive, parse_directive};
pub(crate) use slots::references;
pub(crate) mod strict;
#[cfg(test)]
mod strict_tests;
mod structures;
#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;
pub(crate) mod variants;
#[cfg(test)]
mod variants_tests;

#[derive(Clone, Copy, Debug)]
pub(crate) enum Dialect {
    PostgreSql,
    MySql,
    Sqlite,
}

impl Dialect {
    pub(crate) fn from_engine(engine: &str) -> Result<Self, String> {
        match engine {
            "postgresql" => Ok(Self::PostgreSql),
            "mysql" => Ok(Self::MySql),
            "sqlite" => Ok(Self::Sqlite),
            _ => Err(format!(
                "dynamic filters do not support SQL engine `{engine}`"
            )),
        }
    }

    pub(crate) fn sqlparser(self) -> &'static dyn SqlDialect {
        match self {
            Self::PostgreSql => &PostgreSqlDialect {},
            Self::MySql => &MySqlDialect {},
            Self::Sqlite => &SQLiteDialect {},
        }
    }
}

/// The `dynfilters:` option map.
#[derive(Debug, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct DynFilters {
    pub(crate) prepared: bool,
    pub(crate) prepared_skip: Vec<String>,
    pub(crate) variant_limit: usize,
    pub(crate) variants_skip: Vec<String>,
}

impl Default for DynFilters {
    fn default() -> Self {
        Self {
            prepared: false,
            prepared_skip: Vec::new(),
            variant_limit: 1024,
            variants_skip: Vec::new(),
        }
    }
}

impl DynFilters {
    /// Skip lists need the flag, since enumeration only runs with it on; the enumerator checks
    /// that every listed name is a dynamic query.
    pub(crate) fn validate(&self) -> Result<(), crate::Error> {
        if !self.prepared {
            for (option, list) in [
                ("dynfilters.variants_skip", &self.variants_skip),
                ("dynfilters.prepared_skip", &self.prepared_skip),
            ] {
                if !list.is_empty() {
                    return Err(crate::Error::any(
                        format!("{option} requires dynfilters.prepared: true").into(),
                    ));
                }
            }
        }
        Ok(())
    }
}

static ANNOTATION_START: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"--\s*:").expect("valid annotation start regex"));

/// Cheap pre-check so queries without any `-- :` never reach the tokenizer. It accepts exactly
/// what `strict::annotations` treats as a directive, so a malformed one is always diagnosed.
pub(crate) fn may_contain_annotation(sql: &str) -> bool {
    ANNOTATION_START.is_match(sql)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FlagParam {
    pub(crate) name: String,
    /// Index into `DynFilterInfo::switches` when this slot is one of a switch's choices.
    pub(crate) switch: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Switch {
    pub(crate) field: String,
    pub(crate) choices: Vec<String>,
    pub(crate) default: String,
}

pub(crate) use crate::dynfilter_runtime::Connector;

impl ToTokens for Connector {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        tokens.extend(match self {
            Self::And => quote::quote! {dynfilter::Connector::And},
            Self::Or => quote::quote! {dynfilter::Connector::Or},
            Self::Comma => quote::quote! {dynfilter::Connector::Comma},
        });
    }
}

/// Line indices into `annotated_sql`: the clause header and the first line of each item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlanClause {
    pub(crate) header: usize,
    pub(crate) connector: Connector,
    pub(crate) items: Vec<usize>,
}

impl ToTokens for PlanClause {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let Self {
            header,
            connector,
            items,
        } = self;
        tokens.extend(quote::quote! {
            dynfilter::Clause { header: #header, connector: #connector, items: &[#(#items,)*] }
        });
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DynFilterInfo {
    pub(crate) annotated_sql: String,
    pub(crate) plan: Vec<PlanClause>,
    pub(crate) conditional_param_numbers: Vec<usize>,
    /// Bind slots after the SQL parameters, in annotation order.
    pub(crate) flag_params: Vec<FlagParam>,
    pub(crate) switches: Vec<Switch>,
}

impl DynFilterInfo {
    /// The clause plan in the inlined runtime's types, for compiling the query natively.
    pub(crate) fn runtime_plan(&self) -> Vec<crate::dynfilter_runtime::Clause<'_>> {
        self.plan
            .iter()
            .map(|clause| crate::dynfilter_runtime::Clause {
                header: clause.header,
                connector: clause.connector,
                items: &clause.items,
            })
            .collect()
    }
}

#[derive(Debug)]
pub(crate) struct References {
    pub(crate) conditional_param_numbers: Vec<usize>,
    pub(crate) flag_params: Vec<FlagParam>,
    pub(crate) switches: Vec<Switch>,
}

impl References {
    /// The runtime argument slot a `:if`, `:flag`, or `:case` name gates on: a SQL parameter
    /// sits at `number - 1`, and every flag slot follows the parameters in claim order.
    pub(crate) fn arg_index(&self, params: &[(String, usize)], name: &str) -> Option<usize> {
        params
            .iter()
            .find(|(param, _)| param == name)
            .map(|(_, number)| number - 1)
            .or_else(|| {
                self.flag_params
                    .iter()
                    .position(|flag| flag.name == name)
                    .map(|position| params.len() + position)
            })
    }
}

pub(crate) fn parse_static_slices(
    sql: &str,
    dialect: Dialect,
    params: &[(String, usize)],
) -> Result<Option<DynFilterInfo>, String> {
    if !sql.contains("/*SLICE:") {
        return Ok(None);
    }
    let annotated_sql = number_sqlc_slices(
        sql,
        dialect,
        &params
            .iter()
            .map(|(name, number)| (name.as_str(), *number))
            .collect(),
    )?;
    Ok(Some(DynFilterInfo {
        annotated_sql,
        plan: Vec::new(),
        conditional_param_numbers: Vec::new(),
        flag_params: Vec::new(),
        switches: Vec::new(),
    }))
}

/// Numbers each `/*SLICE:name*/?` marker sqlc emits with the parameter's number so the runtime
/// can find its argument. Only a slice comment token directly followed by a placeholder token
/// counts, so marker-like text inside literals, quoted identifiers, and comments is untouched.
pub(crate) fn number_sqlc_slices(
    sql: &str,
    dialect: Dialect,
    param_by_name: &HashMap<&str, usize>,
) -> Result<String, String> {
    let source = source::Source::tokenize(sql, dialect)?;
    let mut output = String::with_capacity(sql.len());
    let mut cursor = 0;
    let mut pending = None;
    for token in &source.raw {
        match &token.token {
            Token::Whitespace(Whitespace::MultiLineComment(comment)) => {
                pending = comment.strip_prefix("SLICE:");
            }
            Token::Placeholder(_) => {
                if let Some(number) = pending.take().and_then(|name| param_by_name.get(name)) {
                    let range = source.token_range(token);
                    output.push_str(&sql[cursor..range.start]);
                    output.push('?');
                    output.push_str(&number.to_string());
                    cursor = range.end;
                }
            }
            _ => pending = None,
        }
    }
    output.push_str(&sql[cursor..]);
    Ok(output)
}

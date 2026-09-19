use quote::ToTokens;

mod error;
mod names;
mod rows;
mod types;

pub use error::QueryError;
use names::generate_column_names;
#[cfg(test)]
use rows::find_embedded_table;
pub(crate) use rows::{
    ColumnField, ColumnFieldType, DbEnum, EmbeddedTable, ReturnRowAttributes, ReturningRows,
    collect_enums,
};
pub(crate) use types::{
    DbTypeMap, RsColType, RsType, SimpleTypeMap, TypeMapper, make_column_name, make_column_type,
};

use crate::{StackErrorResult, field_ident, plugin};

/// sqlc annotation
/// See https://docs.sqlc.dev/en/stable/reference/query-annotations.html
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Annotation {
    Exec,
    ExecResult,
    ExecRows,
    ExecLastId,
    Many,
    One,
    BatchExec,
    BatchMany,
    BatchOne,
    CopyFrom,
}

impl std::fmt::Display for Annotation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let txt = match self {
            Annotation::Exec => ":exec",
            Annotation::ExecResult => ":execresult",
            Annotation::ExecRows => ":execrows",
            Annotation::ExecLastId => ":execlastid",
            Annotation::Many => ":many",
            Annotation::One => ":one",
            Annotation::BatchExec => ":batch",
            Annotation::BatchMany => ":batchmany",
            Annotation::BatchOne => ":batchone",
            Annotation::CopyFrom => ":copyfrom",
        };
        f.write_str(txt)
    }
}

impl std::str::FromStr for Annotation {
    type Err = QueryError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let annotation = match s {
            ":exec" => Annotation::Exec,
            ":execresult" => Annotation::ExecResult,
            ":execrows" => Annotation::ExecRows,
            ":execlastid" => Annotation::ExecLastId,
            ":many" => Annotation::Many,
            ":one" => Annotation::One,
            ":batch" => Annotation::BatchExec,
            ":batchmany" => Annotation::BatchMany,
            ":batchone" => Annotation::BatchOne,
            ":copyfrom" => Annotation::CopyFrom,
            _ => return Err(QueryError::unknown_annotation(s.to_string())),
        };
        Ok(annotation)
    }
}
/// A raw string literal keeps multi-line SQL readable in generated code. A `"` followed by `n`
/// hashes inside the text needs a delimiter of `n + 1` hashes; text a raw literal cannot hold
/// (a bare carriage return) falls back to an escaped literal.
pub(crate) fn raw_string_literal(s: &str) -> proc_macro2::TokenStream {
    let hashes = "#".repeat(
        s.split('"')
            .skip(1)
            .map(|rest| rest.bytes().take_while(|byte| *byte == b'#').count() + 1)
            .max()
            .unwrap_or(0),
    );
    format!("r{hashes}\"{s}\"{hashes}")
        .parse::<proc_macro2::TokenStream>()
        .unwrap_or_else(|_| proc_macro2::Literal::string(s).to_token_stream())
}

/// Rejects two source names that normalize to one Rust member spelling; `syn` accepts the
/// duplicate, so nothing later would. `owner` names the generated item, e.g. "Query `X`".
pub(crate) fn validate_unique_members(
    owner: impl std::fmt::Display,
    item: &'static str,
    members: impl IntoIterator<Item = (String, String)>,
) -> Result<(), QueryError> {
    let mut sources = std::collections::BTreeMap::new();
    for (ident, source) in members {
        crate::unique::insert_unique(&mut sources, ident.clone(), source.clone()).map_err(
            |first| QueryError::conflicting_member(owner.to_string(), item, first, source, ident),
        )?;
    }
    Ok(())
}

/// sqlc's SQLite engine copies a preceding query's own-line `;` onto the start of the next
/// query's text; every text sqlc reports enters the plugin through here.
pub(crate) fn sqlc_text(text: &str) -> &str {
    text.trim_start_matches(|c: char| c == ';' || c.is_whitespace())
}

pub(crate) struct ParamSlot {
    pub(crate) field_index: usize,
    pub(crate) conditional: bool,
    pub(crate) slice: bool,
}

pub(crate) struct ArgSlots {
    pub(crate) params: Vec<ParamSlot>,
}

impl ArgSlots {
    pub(crate) fn flag_slot(&self, flag_index: usize) -> usize {
        self.params.len() + flag_index
    }
}

pub(crate) struct Query {
    pub(crate) fields: Vec<ColumnField>,

    pub(crate) annotation: Annotation,
    /// ```sql
    /// -- name: GetAuthor :one
    ///          ^^^^^^^^^
    /// SELECT * FROM authors
    /// WHERE id = $1 LIMIT 1;
    /// ```
    pub(crate) query_name: String,
    /// ```sql
    /// -- name: GetAuthor :one
    /// SELECT * FROM authors
    /// ^^^^^^^^^^^^^^^^^^^^^
    /// WHERE id = $1 LIMIT 1;
    /// ^^^^^^^^^^^^^^^^^^^^^^
    /// ```
    /// The text sqlc reported; `sql()` selects the rewritten text once a filter applies.
    query_str: String,
    param_numbers: Vec<usize>,
    sqlc_slice_param_numbers: std::collections::BTreeSet<usize>,
    dynfilter: Option<crate::dynfilter::DynFilterInfo>,
}

impl Query {
    pub(crate) fn from_query(
        db_type: &DbTypeMap,
        query: &plugin::Query,
    ) -> Result<Self, QueryError> {
        let columns = query
            .params
            .iter()
            .map(|p| {
                p.column
                    .as_ref()
                    .ok_or_else(|| QueryError::missing_param_column(p.number))
            })
            .collect::<Result<Vec<_>, _>>()?;

        let field_names = generate_column_names(columns.iter().copied())
            .into_iter()
            .map(|s| field_ident(&s));
        let original_names = columns
            .iter()
            .map(|col| syn::LitStr::new(&col.name, proc_macro2::Span::call_site()));
        let param_names = field_names.zip(original_names);

        let param_types = columns
            .iter()
            .map(|col| RsColType::new_with_type(db_type, col))
            .collect::<Result<Vec<_>, _>>()?;

        let fields = param_names
            .zip(param_types)
            .map(|((par_name, par_name_original), par_type)| ColumnField {
                name: par_name,
                name_original: par_name_original,
                typ: ColumnFieldType::Scalar(Box::new(par_type)),
                attribute: None,
            })
            .collect::<Vec<_>>();

        let annotation = query.cmd.parse::<Annotation>().stacked()?;
        let query_name = query.name.to_string();

        validate_unique_members(
            format_args!("Query `{query_name}`"),
            "parameter",
            fields
                .iter()
                .map(|field| (field.name.to_string(), field.name_original.value())),
        )?;
        let query_str = sqlc_text(&query.text).to_string();
        let param_numbers = query
            .params
            .iter()
            .map(|param| usize::try_from(param.number).unwrap_or_default())
            .collect();
        let sqlc_slice_param_numbers = query
            .params
            .iter()
            .filter_map(|param| {
                param
                    .column
                    .as_ref()
                    .filter(|column| column.is_sqlc_slice)
                    .map(|_| usize::try_from(param.number).unwrap_or_default())
            })
            .collect();

        Ok(Self {
            fields,
            annotation,
            query_name,
            query_str,
            param_numbers,
            sqlc_slice_param_numbers,
            dynfilter: None,
        })
    }

    /// Dynamic-filter annotations, then static `sqlc.slice` expansion for backends that inline
    /// slices into queries without annotations.
    pub(crate) fn parse(
        db_type: &DbTypeMap,
        query: &plugin::Query,
        dialect: crate::dynfilter::Dialect,
        catalog: &crate::dynfilter::resolve::Catalog<'_>,
        static_slices: bool,
    ) -> Result<Self, QueryError> {
        let mut parsed = Self::from_query(db_type, query)?;
        parsed.apply_dynfilter(dialect, catalog)?;
        if static_slices && parsed.dynfilter.is_none() {
            parsed.apply_static_slices(dialect)?;
        }
        Ok(parsed)
    }

    fn apply_dynfilter(
        &mut self,
        dialect: crate::dynfilter::Dialect,
        catalog: &crate::dynfilter::resolve::Catalog<'_>,
    ) -> Result<(), QueryError> {
        let params = self.params();
        let Some(info) =
            crate::dynfilter::strict::parse(&self.query_str, &params, dialect, catalog)
                .map_err(|message| QueryError::dynamic_filter(self.query_name.clone(), message))?
        else {
            return Ok(());
        };
        for (field, number) in self.fields.iter_mut().zip(&self.param_numbers) {
            if info.conditional_param_numbers.contains(number) {
                field.scalar_type_mut().make_optional();
            }
        }
        self.dynfilter = Some(info);
        Ok(())
    }

    pub(crate) fn apply_static_slices(
        &mut self,
        dialect: crate::dynfilter::Dialect,
    ) -> Result<(), QueryError> {
        let params = self.params();
        let Some(info) =
            crate::dynfilter::parse_static_slices(&self.query_str, dialect, &params)
                .map_err(|message| QueryError::dynamic_filter(self.query_name.clone(), message))?
        else {
            return Ok(());
        };
        self.dynfilter = Some(info);
        Ok(())
    }

    pub(crate) fn dynfilter(&self) -> Option<&crate::dynfilter::DynFilterInfo> {
        self.dynfilter.as_ref()
    }

    /// Whether the query has `:if`, `:flag`, or `:switch` controls; static slices alone do not
    /// count, and neither does a plain query.
    pub(crate) fn has_dynamic_controls(&self) -> bool {
        self.dynfilter().is_some_and(|info| {
            !info.conditional_param_numbers.is_empty() || !info.flag_params.is_empty()
        })
    }

    /// For code paths that only run once `dynfilter()` is known to be `Some`.
    pub(crate) fn expect_dynfilter(&self) -> &crate::dynfilter::DynFilterInfo {
        self.dynfilter().expect("dynamic query")
    }

    /// The runtime `args` layout: SQL parameters ordered by number, then one flag slot per
    /// `flag_params` entry. The generated caller, the variant expander, and the test compiler
    /// all read it from here so they can never disagree on which slot a control owns.
    pub(crate) fn arg_slots(&self) -> ArgSlots {
        let info = self.expect_dynfilter();
        let mut params = self
            .param_numbers
            .iter()
            .enumerate()
            .map(|(field_index, number)| {
                (
                    *number,
                    ParamSlot {
                        field_index,
                        conditional: info.conditional_param_numbers.contains(number),
                        slice: self.sqlc_slice_param_numbers.contains(number),
                    },
                )
            })
            .collect::<Vec<_>>();
        params.sort_unstable_by_key(|(number, _)| *number);
        ArgSlots {
            params: params.into_iter().map(|(_, slot)| slot).collect(),
        }
    }

    pub(crate) fn param_number(&self, field_index: usize) -> usize {
        self.param_numbers[field_index]
    }

    /// SQL parameter numbers in field order, as the runtime's `compile_with_arg_order` wants.
    pub(crate) fn arg_order(&self) -> Vec<usize> {
        self.param_numbers.clone()
    }

    pub(crate) fn is_sqlc_slice(&self, field_index: usize) -> bool {
        self.sqlc_slice_param_numbers
            .contains(&self.param_number(field_index))
    }

    pub(crate) fn params_need_lifetime(&self) -> bool {
        self.fields
            .iter()
            .any(|field| field.scalar_type().need_params_struct_lifetime())
    }

    fn params(&self) -> Vec<(String, usize)> {
        self.fields
            .iter()
            .zip(&self.param_numbers)
            .map(|(field, number)| (field.name_original.value(), *number))
            .collect()
    }

    /// The SQL the generated constant carries: the rewritten text once dynamic filters or
    /// static slices apply, otherwise sqlc's.
    pub(crate) fn sql(&self) -> &str {
        self.dynfilter
            .as_ref()
            .map_or(&self.query_str, |info| &info.annotated_sql)
    }

    pub(crate) fn query_str(&self) -> proc_macro2::TokenStream {
        raw_string_literal(self.sql())
    }
}

#[cfg(test)]
mod tests;

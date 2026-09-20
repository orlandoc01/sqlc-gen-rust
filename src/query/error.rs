use crate::{StackError, StackErrorResult, plugin};

#[derive(Debug, Clone)]
pub struct QueryError {
    kind: QueryErrorKind,
    location: &'static std::panic::Location<'static>,
}

#[derive(Debug, Clone)]
pub(crate) enum QueryErrorKind {
    MissingColumnType {
        column_name: String,
    },
    MissingParamColumn {
        param_number: i32,
    },
    CannotMapType {
        message: String,
    },
    MissingEmbeddedTable {
        table_name: String,
    },
    ConflictingEmbeddedTable {
        first_table_name: String,
        second_table_name: String,
        struct_ident: String,
    },
    ConflictingGeneratedItem(Box<GeneratedItemConflict>),
    UnsupportedArrayDimensions {
        query_name: String,
        column_name: String,
    },
    UnknownAnnotation {
        annotation: String,
    },
    DynamicFilter {
        query_name: String,
        message: String,
    },
    /// Two source names normalize to one Rust member spelling inside one generated item.
    ConflictingMember(Box<MemberConflict>),
    Stacked(Box<QueryError>),
}

#[derive(Debug, Clone)]
pub struct MemberConflict {
    owner: String,
    item: &'static str,
    first: String,
    second: String,
    ident: String,
}

#[derive(Debug, Clone)]
pub struct GeneratedItemConflict {
    first_query_name: String,
    first_helper: &'static str,
    second_query_name: String,
    second_helper: &'static str,
    function_ident: String,
}

impl QueryError {
    #[track_caller]
    fn new(kind: QueryErrorKind) -> Self {
        Self {
            kind,
            location: std::panic::Location::caller(),
        }
    }

    #[track_caller]
    pub(crate) fn missing_column_type(column_name: String) -> Self {
        Self::new(QueryErrorKind::MissingColumnType { column_name })
    }

    #[track_caller]
    pub(crate) fn missing_param_column(param_number: i32) -> Self {
        Self::new(QueryErrorKind::MissingParamColumn { param_number })
    }

    #[track_caller]
    pub(crate) fn cannot_map_type(col_name: String, typ_name: String) -> Self {
        Self::new(QueryErrorKind::CannotMapType {
            message: format!(
                "Cannot map type `{col_name}` of table `{typ_name}` to a Rust type. Consider add entry to overrides."
            ),
        })
    }

    #[track_caller]
    pub(crate) fn missing_embedded_table(table: &plugin::Identifier) -> Self {
        let table_name = if table.schema.is_empty() {
            table.name.clone()
        } else {
            format!("{}.{}", table.schema, table.name)
        };
        Self::new(QueryErrorKind::MissingEmbeddedTable { table_name })
    }

    #[track_caller]
    pub(crate) fn conflicting_embedded_table(
        first_table_name: String,
        second_table_name: String,
        struct_ident: String,
    ) -> Self {
        Self::new(QueryErrorKind::ConflictingEmbeddedTable {
            first_table_name,
            second_table_name,
            struct_ident,
        })
    }

    #[track_caller]
    pub(crate) fn conflicting_generated_function(
        first_query_name: String,
        first_helper: &'static str,
        second_query_name: String,
        second_helper: &'static str,
        function_ident: String,
    ) -> Self {
        Self::new(QueryErrorKind::ConflictingGeneratedItem(Box::new(
            GeneratedItemConflict {
                first_query_name,
                first_helper,
                second_query_name,
                second_helper,
                function_ident,
            },
        )))
    }

    #[track_caller]
    pub(crate) fn unsupported_array_dimensions(query_name: String, column_name: String) -> Self {
        Self::new(QueryErrorKind::UnsupportedArrayDimensions {
            query_name,
            column_name,
        })
    }

    #[track_caller]
    pub(crate) fn unknown_annotation(annotation: String) -> Self {
        Self::new(QueryErrorKind::UnknownAnnotation { annotation })
    }

    #[track_caller]
    pub(crate) fn conflicting_member(
        owner: String,
        item: &'static str,
        first: String,
        second: String,
        ident: String,
    ) -> Self {
        Self::new(QueryErrorKind::ConflictingMember(Box::new(
            MemberConflict {
                owner,
                item,
                first,
                second,
                ident,
            },
        )))
    }

    #[track_caller]
    pub(crate) fn dynamic_filter(query_name: String, message: String) -> Self {
        Self::new(QueryErrorKind::DynamicFilter {
            query_name,
            message,
        })
    }
}

impl std::fmt::Display for QueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.kind {
            QueryErrorKind::MissingColumnType { column_name } => {
                write!(f, "Column type not found for column: `{column_name}`")
            }
            QueryErrorKind::MissingParamColumn { param_number } => {
                write!(
                    f,
                    "Parameter column not found for parameter #{param_number}"
                )
            }
            QueryErrorKind::UnknownAnnotation { annotation } => {
                write!(f, "Unknown annotation `{annotation}` found")
            }
            QueryErrorKind::DynamicFilter {
                query_name,
                message,
            } => write!(f, "Query `{query_name}`: {message}"),
            QueryErrorKind::ConflictingMember(conflict) => write!(
                f,
                "{}: `{}` and `{}` both generate {} `{}`",
                conflict.owner, conflict.first, conflict.second, conflict.item, conflict.ident
            ),
            QueryErrorKind::CannotMapType { message } => message.fmt(f),
            QueryErrorKind::MissingEmbeddedTable { table_name } => {
                write!(f, "Embedded table not found in catalog: `{table_name}`")
            }
            QueryErrorKind::ConflictingEmbeddedTable {
                first_table_name,
                second_table_name,
                struct_ident,
            } => write!(
                f,
                "Embedded tables `{first_table_name}` and `{second_table_name}` both generate Rust struct `{struct_ident}`"
            ),
            QueryErrorKind::ConflictingGeneratedItem(conflict) => write!(
                f,
                "Queries `{}` ({}) and `{}` ({}) both generate Rust item `{}`",
                conflict.first_query_name,
                conflict.first_helper,
                conflict.second_query_name,
                conflict.second_helper,
                conflict.function_ident,
            ),
            QueryErrorKind::UnsupportedArrayDimensions {
                query_name,
                column_name,
            } => write!(
                f,
                "PostgreSQL backend supports one-dimensional arrays only: query `{query_name}`, column `{column_name}`"
            ),
            QueryErrorKind::Stacked(source) => source.fmt(f),
        }
    }
}

impl StackError for QueryError {
    fn format_stack(&self, layer: usize, buf: &mut Vec<String>) {
        let (file, line) = (self.location.file(), self.location.line());
        buf.push(match &self.kind {
            QueryErrorKind::Stacked(_) => format!("{layer}: at {file}:{line}"),
            _ => format!("{layer}:{self} , at {file}:{line}"),
        });
    }

    fn next(&self) -> Option<&dyn StackError> {
        match &self.kind {
            QueryErrorKind::Stacked(source) => Some(source.as_ref()),
            _ => None,
        }
    }
}

impl std::error::Error for QueryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.kind {
            QueryErrorKind::Stacked(source) => Some(source.as_ref()),
            _ => None,
        }
    }
}

impl<T> StackErrorResult<T, QueryError> for Result<T, QueryError> {
    #[track_caller]
    fn stacked(self) -> Self {
        self.map_err(|err| QueryError::new(QueryErrorKind::Stacked(Box::new(err))))
    }
}

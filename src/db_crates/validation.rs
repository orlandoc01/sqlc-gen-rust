use crate::query::{self, ColumnField, ColumnFieldType, ReturningRows};

use super::DbCrate;

impl DbCrate {
    pub(crate) fn validate_array_dimensions(
        self,
        rows: &[ReturningRows],
        queries: &[query::Query],
    ) -> Result<(), query::QueryError> {
        if !matches!(self, Self::Postgres(_)) {
            return Ok(());
        }

        let invalid_parameter = queries.iter().find_map(|query| {
            query
                .fields
                .iter()
                .find(|field| field.scalar_type().array_dimensions() > 1)
                .map(|field| (&query.query_name, field))
        });
        let invalid_returning = rows.iter().find_map(|row| {
            row.fields
                .iter()
                .find_map(unsupported_array_field)
                .map(|field| (&row.query_name, field))
        });
        let Some((query_name, field)) = invalid_parameter.or(invalid_returning) else {
            return Ok(());
        };

        Err(query::QueryError::unsupported_array_dimensions(
            query_name.clone(),
            field.name_original.value(),
        ))
    }
}

fn unsupported_array_field(field: &ColumnField) -> Option<&ColumnField> {
    match &field.typ {
        ColumnFieldType::Scalar(typ) if typ.array_dimensions() > 1 => Some(field),
        ColumnFieldType::Scalar(_) => None,
        ColumnFieldType::Embed(table) => table.fields.iter().find_map(unsupported_array_field),
    }
}

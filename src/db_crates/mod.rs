use crate::query::{self, EmbeddedTable, ReturningRows};

mod sqlx;
pub(crate) mod sqlx_params;

pub(crate) use sqlx::Sqlx;

fn make_return_row(row: &query::ReturningRows) -> proc_macro2::TokenStream {
    let ident = &row.struct_ident();
    make_struct(ident, &row.attributes, &row.fields)
}

fn make_embedded_table(table: &EmbeddedTable) -> proc_macro2::TokenStream {
    make_struct(&table.ident, &table.attributes, &table.fields)
}

pub(crate) fn make_embedded_tables(
    rows: &[ReturningRows],
) -> Result<proc_macro2::TokenStream, query::QueryError> {
    let mut tables = std::collections::BTreeMap::new();
    let mut idents = std::collections::BTreeMap::new();
    for table in rows.iter().flat_map(ReturningRows::embedded_tables) {
        if tables.contains_key(&table.qualified_name) {
            continue;
        }

        let ident = table.ident.to_string();
        if let Some(existing_table) = idents.insert(ident.clone(), &table.qualified_name) {
            return Err(query::QueryError::conflicting_embedded_table(
                existing_table.clone(),
                table.qualified_name.clone(),
                ident,
            ));
        }
        tables.insert(table.qualified_name.clone(), table);
    }

    let tables = tables.values().map(|table| make_embedded_table(table));
    Ok(quote::quote! {#(#tables)*})
}

fn make_struct(
    ident: &syn::Ident,
    attributes: &Option<proc_macro2::TokenStream>,
    column_fields: &[query::ColumnField],
) -> proc_macro2::TokenStream {
    let fields = column_fields.iter().map(|field| {
        let field_name = &field.name;
        let field_typ = field.row_type();
        let attribute = &field.attribute;
        quote::quote! {
            #attribute
            pub #field_name:#field_typ
        }
    });
    quote::quote! {
        #attributes
        pub struct #ident {
            #(#fields,)*
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(table: EmbeddedTable) -> ReturningRows {
        ReturningRows {
            fields: vec![query::ColumnField {
                name: crate::field_ident("users"),
                name_original: syn::LitStr::new("users", proc_macro2::Span::call_site()),
                typ: query::ColumnFieldType::Embed(table),
                attribute: None,
            }],
            query_name: String::new(),
            attributes: None,
        }
    }

    fn embedded_table(qualified_name: &str) -> EmbeddedTable {
        EmbeddedTable {
            qualified_name: qualified_name.to_string(),
            ident: crate::value_ident("users"),
            fields: Vec::new(),
            attributes: None,
        }
    }

    #[test]
    fn rejects_embedded_tables_with_conflicting_struct_idents() {
        let error = make_embedded_tables(&[
            row(embedded_table("first.users")),
            row(embedded_table("second.users")),
        ])
        .unwrap_err();

        assert_eq!(
            error.to_string(),
            "Embedded tables `first.users` and `second.users` both generate Rust struct `Users`"
        );
    }
}

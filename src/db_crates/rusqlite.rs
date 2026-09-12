use crate::query::{ColumnField, DbEnum, ReturningRows};

use super::{make_return_row, sqlx::Sqlx};

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Rusqlite;

impl Rusqlite {
    pub(crate) fn db_type_map(self) -> crate::query::DbTypeMap {
        Sqlx::Sqlite.db_type_map()
    }

    pub(crate) fn init(self) -> proc_macro2::TokenStream {
        quote::quote! {
            pub trait RusqliteClient {
                fn connection(&self) -> &rusqlite::Connection;
            }

            impl RusqliteClient for rusqlite::Connection {
                fn connection(&self) -> &rusqlite::Connection {
                    self
                }
            }

            impl RusqliteClient for rusqlite::Transaction<'_> {
                fn connection(&self) -> &rusqlite::Connection {
                    self
                }
            }

            impl RusqliteClient for rusqlite::Savepoint<'_> {
                fn connection(&self) -> &rusqlite::Connection {
                    self
                }
            }
        }
    }

    pub(crate) fn defined_enum(self, _enum_type: &DbEnum) -> proc_macro2::TokenStream {
        proc_macro2::TokenStream::new()
    }

    pub(crate) fn returning_ordinal_row(self, row: &ReturningRows) -> proc_macro2::TokenStream {
        let struct_tokens = make_return_row(row);
        let ident = row.struct_ident();
        let row_ident = quote::format_ident!("row");
        let fields = row
            .fields
            .iter()
            .zip(row.field_ordinals())
            .map(|(field, ordinal)| Self::field_from_row(field, &row_ident, ordinal));
        quote::quote! {
            #struct_tokens
            impl #ident {
                pub fn from_row(#row_ident: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
                    Ok(Self { #(#fields,)* })
                }
            }
        }
    }

    fn field_from_row(
        field: &ColumnField,
        row: &syn::Ident,
        ordinal: std::ops::Range<usize>,
    ) -> proc_macro2::TokenStream {
        let field_ident = &field.name;
        let literal = proc_macro2::Literal::usize_unsuffixed(ordinal.start);
        match field.embedded_table() {
            None => quote::quote! {#field_ident: #row.get(#literal)?},
            Some(table) => {
                let table_ident = &table.ident;
                let fields = table.fields.iter().zip(ordinal).map(|(field, index)| {
                    let field_ident = &field.name;
                    let literal = proc_macro2::Literal::usize_unsuffixed(index);
                    quote::quote! {#field_ident: #row.get(#literal)?}
                });
                quote::quote! {#field_ident: #table_ident { #(#fields,)* }}
            }
        }
    }
}

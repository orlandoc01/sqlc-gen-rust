use crate::query::{DbEnum, ReturningRows};

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
        let fields = super::row_field_initializers(row, |index| quote::quote! { row.get(#index)? });
        quote::quote! {
            #struct_tokens
            impl #ident {
                pub fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
                    Ok(Self { #(#fields,)* })
                }
            }
        }
    }
}

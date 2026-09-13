use crate::query::DbEnum;

use super::sqlx::Sqlx;

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
}

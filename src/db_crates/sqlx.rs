use crate::{
    query::{DbEnum, Query, ReturningRows, RsType, SimpleTypeMap, TypeMapper},
    value_ident,
};

#[derive(Default)]
pub struct MySqlTypeMap {
    type_map: std::collections::BTreeMap<String, RsType>,
}

impl TypeMapper for MySqlTypeMap {
    fn find_rs_type(&self, db_type_name: &str) -> Option<&RsType> {
        self.type_map.get(db_type_name)
    }

    fn find_column_type(&self, column: &crate::plugin::Column) -> Option<RsType> {
        let col_type = column
            .r#type
            .as_ref()
            .map(crate::query::make_column_type)?
            .to_lowercase();
        if let Some(rs_type) = self.find_rs_type(&col_type) {
            return Some(rs_type.clone());
        };

        match col_type.as_str() {
            "tinyint" => match (column.length, column.unsigned) {
                (1, _) => Some(RsType::new(syn::parse_str("bool").unwrap(), None, true)),
                (_, true) => Some(RsType::new(syn::parse_str("u8").unwrap(), None, true)),
                (_, false) => Some(RsType::new(syn::parse_str("i8").unwrap(), None, true)),
            },
            "smallint" => Some(RsType::new(
                syn::parse_str(if column.unsigned { "u16" } else { "i16" }).unwrap(),
                None,
                true,
            )),
            "int" | "integer" | "mediumint" => Some(RsType::new(
                syn::parse_str(if column.unsigned { "u32" } else { "i32" }).unwrap(),
                None,
                true,
            )),
            "bigint" => Some(RsType::new(
                syn::parse_str(if column.unsigned { "u64" } else { "i64" }).unwrap(),
                None,
                true,
            )),
            _ => None,
        }
    }

    fn insert_db_type(&mut self, db_type: &str, rs_type: RsType) {
        self.type_map.insert(db_type.to_string(), rs_type);
    }
}

#[derive(Default)]
pub struct SqliteTypeMap {
    type_map: std::collections::BTreeMap<String, RsType>,
}

impl TypeMapper for SqliteTypeMap {
    fn find_rs_type(&self, db_type_name: &str) -> Option<&RsType> {
        self.type_map.get(db_type_name)
    }

    fn find_column_type(&self, column: &crate::plugin::Column) -> Option<RsType> {
        let col_type = column
            .r#type
            .as_ref()
            .map(crate::query::make_column_type)?
            .to_lowercase();
        if let Some(rs_type) = self.find_rs_type(&col_type) {
            return Some(rs_type.clone());
        };

        // Rust type determined by affinity
        // See https://www.sqlite.org/datatype3.html
        if col_type.contains("int") {
            return self.find_rs_type("int").cloned();
        }
        if col_type.contains("char") || col_type.contains("clob") || col_type.contains("text") {
            return self.find_rs_type("text").cloned();
        }
        if col_type.contains("blob") || col_type.is_empty() {
            return self.find_rs_type("blob").cloned();
        }
        if col_type.contains("real") || col_type.contains("floa") || col_type.contains("doub") {
            return self.find_rs_type("real").cloned();
        }
        self.find_rs_type("numeric").cloned()
    }

    fn insert_db_type(&mut self, db_type: &str, rs_type: RsType) {
        self.type_map.insert(db_type.to_string(), rs_type);
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) enum Sqlx {
    #[default]
    Postgres,
    MySql,
    Sqlite,
}

impl Sqlx {
    pub(crate) fn db_type_map(&self) -> crate::query::DbTypeMap {
        let copy_cheap = self.copy_cheap_types();
        let default_types = self.default_types();
        let map: Box<dyn TypeMapper> = match self {
            Self::Postgres => Box::new(SimpleTypeMap::default()),
            Self::MySql => Box::new(MySqlTypeMap::default()),
            Self::Sqlite => Box::new(SqliteTypeMap::default()),
        };

        type_map(map, copy_cheap, default_types)
    }

    pub(crate) fn defined_enum(&self, enum_type: &DbEnum) -> proc_macro2::TokenStream {
        let derives = &enum_type.derives;
        let fields = enum_type.values.iter().map(|field| {
            let ident = value_ident(field);
            quote::quote! {
                #[sqlx(rename = #field)]
                #ident
            }
        });
        let original_name = &enum_type.name;
        let enum_name = enum_type.ident();
        let derive = if derives.is_empty() {
            quote::quote! {#[derive(Debug,Clone,Copy, sqlx::Type)]}
        } else {
            quote::quote! {#[derive(Debug,Clone,Copy, sqlx::Type, #(#derives),*)]}
        };
        quote::quote! {
            #derive
            #[sqlx(type_name = #original_name)]
            pub enum #enum_name {
                #(#fields,)*
            }
        }
    }

    pub(crate) fn returning_ordinal_row(&self, row: &ReturningRows) -> proc_macro2::TokenStream {
        let struct_tokens = super::make_return_row(row);
        let ident = row.struct_ident();
        let row_type = self.row_type();
        let fields = super::row_field_initializers(row, |index| {
            quote::quote! { sqlx::Row::try_get(row, #index)? }
        });
        quote::quote! {
            #struct_tokens
            impl<'r> sqlx::FromRow<'r, #row_type> for #ident {
                fn from_row(row: &'r #row_type) -> Result<Self, sqlx::Error> {
                    Ok(Self { #(#fields,)* })
                }
            }
        }
    }

    fn copy_cheap_types(&self) -> &[(&str, &[&str])] {
        match self {
            Self::Postgres => &[
                ("i8", &["char"]),
                ("i16", &["smallint", "int2", "pg_catalog.int2"]),
                ("i32", &["serial", "serial4", "pg_catalog.serial4"]),
                ("i64", &["bigserial", "serial8", "pg_catalog.serial8"]),
                ("i16", &["smallserial", "serial2", "pg_catalog.serial2"]),
                ("i32", &["integer", "int", "int4", "pg_catalog.int4"]),
                ("i64", &["bigint", "int8", "pg_catalog.int8"]),
                (
                    "f64",
                    &["float", "double precision", "float8", "pg_catalog.float8"],
                ),
                ("f32", &["real", "float4", "pg_catalog.float4"]),
                ("bool", &["boolean", "bool", "pg_catalog.bool"]),
                ("sqlx::postgres::types::Oid", &["oid", "pg_catalog.oid"]),
                ("uuid::Uuid", &["uuid"]),
            ],
            Self::MySql => &[
                ("bool", &["bool", "boolean"]),
                // int types are handled in `find_column_type`
                ("int16", &["year"]),
                ("f32", &["float"]),
                ("f64", &["double", "double precision", "real"]),
                ("sqlx::mysql::types::MySqlTime", &["time"]),
            ],
            Self::Sqlite => &[
                ("bool", &["bool", "boolean"]),
                ("i8", &["tinyint"]),
                ("i16", &["smallint", "int2"]),
                ("i32", &["mediumint", "int4"]),
                ("i64", &["int", "integer", "bigint", "int8"]),
                ("f64", &["real", "double", "doubleprecision", "float"]),
                // NUMERIC affinity
                ("f64", &["numeric"]),
            ],
        }
    }

    fn default_types(&self) -> &[(&str, Option<&str>, &[&str])] {
        match self {
            // https://github.com/sqlc-dev/sqlc/blob/v1.29.0/internal/codegen/golang/postgresql_type.go#L37-L605
            // https://docs.rs/sqlx/latest/sqlx/postgres/types/index.html
            Self::Postgres => &[
                (
                    "String",
                    Some("str"),
                    &[
                        "text",
                        "pg_catalog.varchar",
                        "pg_catalog.bpchar",
                        "string",
                        "citext",
                        "name",
                    ],
                ),
                (
                    "Vec<u8>",
                    Some("[u8]"),
                    &["bytea", "blob", "pg_catalog.bytea"],
                ),
                (
                    "sqlx::postgres::types::PgInterval",
                    None,
                    &["interval", "pg_catalog.interval"],
                ),
                // TODO: Add PgRange<T>
                // https://github.com/sqlc-dev/sqlc/blob/v1.29.0/internal/codegen/golang/postgresql_type.go#L355-L461
                ("sqlx::postgres::types::PgMoney", None, &["money"]),
                ("sqlx::postgres::types::PgLTree", None, &["ltree"]),
                ("sqlx::postgres::types::PgLQuery", None, &["lquery"]),
                ("sqlx::postgres::types::PgCube", None, &["cube"]),
                ("sqlx::postgres::types::PgPoint", None, &["point"]),
                ("sqlx::postgres::types::PgLine", None, &["line"]),
                ("sqlx::postgres::types::PgLSeg", None, &["lseg"]),
                ("sqlx::postgres::types::PgBox", None, &["box"]),
                ("sqlx::postgres::types::PgPath", None, &["path"]),
                ("sqlx::postgres::types::PgPolygon", None, &["polygon"]),
                ("sqlx::postgres::types::PgCircle", None, &["circle"]),
                ("sqlx::postgres::types::PgHstore", None, &["hstore"]),
                (
                    "sqlx::postgres::types::PgTimeTz",
                    None,
                    &["pg_catalog.timetz"],
                ),
                ("std::net::IpAddr", None, &["inet"]),
                (
                    "serde_json::Value",
                    None,
                    &["json", "pg_catalog.json", "jsonb", "pg_catalog.jsonb"],
                ),
            ],
            // https://github.com/sqlc-dev/sqlc/blob/v1.29.0/internal/codegen/golang/mysql_type.go
            // https://docs.rs/sqlx/0.8.6/sqlx/mysql/types/index.html
            Self::MySql => &[
                (
                    "String",
                    Some("str"),
                    &[
                        "varchar",
                        "text",
                        "char",
                        "tinytext",
                        "mediumtext",
                        "longtext",
                    ],
                ),
                (
                    "Vec<u8>",
                    Some("[u8]"),
                    &[
                        "blob",
                        "binary",
                        "varbinary",
                        "tinyblob",
                        "mediumblob",
                        "longblob",
                    ],
                ),
                ("serde_json::Value", None, &["json"]),
                ("String", Some("str"), &["decimal", "dec", "fixed", "enum"]),
            ],
            // https://github.com/sqlc-dev/sqlc/blob/v1.29.0/internal/codegen/golang/sqlite_type.go
            // https://docs.rs/sqlx/latest/sqlx/sqlite/types/index.html
            Self::Sqlite => &[
                ("String", Some("str"), &["text", "clob"]),
                ("Vec<u8>", Some("[u8]"), &["blob"]),
            ],
        }
    }

    pub(crate) fn database_ident(&self) -> syn::Type {
        match self {
            Self::Postgres => syn::parse_quote! {sqlx::Postgres},
            Self::MySql => syn::parse_quote! {sqlx::MySql},
            Self::Sqlite => syn::parse_quote! {sqlx::Sqlite},
        }
    }

    fn row_type(&self) -> syn::Type {
        match self {
            Self::Postgres => syn::parse_quote! {sqlx::postgres::PgRow},
            Self::MySql => syn::parse_quote! {sqlx::mysql::MySqlRow},
            Self::Sqlite => syn::parse_quote! {sqlx::sqlite::SqliteRow},
        }
    }

    pub(crate) fn query_bind<F>(
        &self,
        query: &Query,
        query_ident: syn::Ident,
        accessor: F,
    ) -> proc_macro2::TokenStream
    where
        F: Fn(&syn::Ident) -> proc_macro2::TokenStream,
    {
        match self {
            Self::Postgres => query.fields.iter().map(|field| {
                let value = accessor(&field.name);
                quote::quote! { let #query_ident = #query_ident.bind(#value); }
            }).collect(),
            Self::MySql | Self::Sqlite => query.fields.iter().map(|field| {
                let value = accessor(&field.name);
                if field.scalar_type().is_array() {
                    quote::quote! { let #query_ident = #value.iter().fold(#query_ident, |q, item| q.bind(item)); }
                } else {
                    quote::quote! { let #query_ident = #query_ident.bind(#value); }
                }
            }).collect(),
        }
    }
}

fn type_map(
    mut map: Box<dyn TypeMapper>,
    copy_cheap_types: &[(&str, &[&str])],
    default_types: &[(&str, Option<&str>, &[&str])],
) -> crate::query::DbTypeMap {
    for (owned_type, db_types) in copy_cheap_types {
        let owned_type = syn::parse_str::<syn::Type>(owned_type).expect("Failed to parse type");
        for db_type in *db_types {
            map.insert_db_type(db_type, RsType::new(owned_type.clone(), None, true));
        }
    }
    for (owned_type, slice_type, db_types) in default_types {
        let owned_type = syn::parse_str::<syn::Type>(owned_type).expect("Failed to parse type");
        let slice_type = slice_type
            .map(|typ| syn::parse_str::<syn::Type>(typ).expect("Failed to parse slice type"));
        for db_type in *db_types {
            map.insert_db_type(
                db_type,
                RsType::new(owned_type.clone(), slice_type.clone(), false),
            );
        }
    }
    crate::query::DbTypeMap::from_dyn(map)
}

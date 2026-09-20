use quote::ToTokens;

use crate::{StackErrorResult, plugin};

use super::QueryError;

#[derive(Clone)]
pub(crate) struct RsType {
    owned: syn::Type,
    slice: Option<syn::Type>,
    copy_cheap: bool,
    /// Set by an override; the built-in list in `RsColType::can_default` covers std types.
    can_default: bool,
}

impl RsType {
    pub(crate) fn new(owned: syn::Type, slice: Option<syn::Type>, copy_cheap: bool) -> Self {
        RsType {
            owned,
            slice,
            copy_cheap,
            can_default: false,
        }
    }

    pub(crate) fn with_can_default(self, can_default: bool) -> Self {
        Self {
            can_default,
            ..self
        }
    }

    #[cfg(test)]
    pub(crate) fn can_default(&self) -> bool {
        self.can_default
    }

    /// 自己所有の型を返す
    pub(crate) fn owned(&self) -> proc_macro2::TokenStream {
        self.owned.to_token_stream()
    }

    /// スライスの型を返す。これに`&`をつけると参照になる
    pub(crate) fn slice(&self) -> proc_macro2::TokenStream {
        if let Some(ref slice) = self.slice {
            slice.to_token_stream()
        } else {
            self.owned()
        }
    }
}

#[derive(Clone)]
pub(crate) struct RsColType {
    rs_type: RsType,
    /// maybe dim
    dim: usize,
    /// col is optional
    optional: bool,
}
pub(crate) fn make_column_type(db_type: &plugin::Identifier) -> String {
    if !db_type.schema.is_empty() {
        format!("{}.{}", db_type.schema, db_type.name)
    } else {
        db_type.name.to_string()
    }
}

pub(crate) fn make_column_name(column: &plugin::Column) -> String {
    if let Some(table) = &column.table {
        format!(".{}.{}", table.name, column.name)
    } else {
        format!(".{}", column.name)
    }
}

impl RsColType {
    pub(crate) fn is_array(&self) -> bool {
        self.dim != 0
    }

    pub(crate) fn array_dimensions(&self) -> usize {
        self.dim
    }

    pub(crate) fn make_optional(&mut self) {
        self.optional = true;
    }

    pub(crate) fn can_default(&self) -> bool {
        if self.optional || self.need_params_struct_lifetime() || self.rs_type.can_default {
            return true;
        }

        matches!(
            self.rs_type.owned.to_token_stream().to_string().as_str(),
            "bool"
                | "i8"
                | "i16"
                | "i32"
                | "i64"
                | "i128"
                | "isize"
                | "u8"
                | "u16"
                | "u32"
                | "u64"
                | "u128"
                | "usize"
                | "f32"
                | "f64"
                | "String"
                | "uuid :: Uuid"
                | "serde_json :: Value"
        )
    }

    pub(crate) fn new_with_type(
        db_type: &DbTypeMap,
        column: &plugin::Column,
    ) -> Result<Self, QueryError> {
        let rs_type = db_type.get_column_type(column).stacked()?;
        let dim = if column.is_sqlc_slice {
            1
        } else {
            usize::try_from(column.array_dims).unwrap_or_default()
        };

        // sqlc.slice parameters are never optional.
        // https://docs.sqlc.dev/en/latest/howto/select.html#mysql-and-sqlite
        let optional = !column.is_sqlc_slice && !column.not_null;

        Ok(Self {
            rs_type,
            dim,
            optional,
        })
    }

    /// Convert to tokens for row struct
    pub(crate) fn to_row_tokens(&self) -> proc_macro2::TokenStream {
        let base_type = self.rs_type.owned();

        // 配列の次元数に応じてVecでラップ
        let mut wrapped_type = base_type;
        for _ in 0..self.dim {
            wrapped_type = quote::quote! { Vec<#wrapped_type> };
        }

        // optionalの場合はOptionでラップ
        if self.optional {
            quote::quote! { Option<#wrapped_type> }
        } else {
            wrapped_type
        }
    }

    pub(crate) fn need_params_struct_lifetime(&self) -> bool {
        self.dim != 0 || self.rs_type.slice.is_some()
    }

    pub(crate) fn copy_cheap(&self) -> bool {
        self.rs_type.copy_cheap
    }

    /// The type the generated code hands to a dynamic `.bind()`: the params-struct type, minus
    /// the `Option` a conditional parameter unwraps before binding.
    pub(crate) fn to_bound_tokens(&self, conditional: bool) -> proc_macro2::TokenStream {
        self.struct_tokens(self.optional && !conditional, None)
    }

    pub(crate) fn to_params_struct_tokens(
        &self,
        lifetime: Option<&syn::Lifetime>,
    ) -> proc_macro2::TokenStream {
        self.struct_tokens(self.optional, lifetime)
    }

    fn struct_tokens(
        &self,
        optional: bool,
        lifetime: Option<&syn::Lifetime>,
    ) -> proc_macro2::TokenStream {
        let wrapped_type = match self.dim {
            0 if self.rs_type.slice.is_some() => self.rs_type.slice(),
            0 => self.rs_type.owned(),
            _ => {
                let mut base_type = self.rs_type.owned();
                for _ in 1..self.dim {
                    base_type = quote::quote! {Vec<#base_type>};
                }
                quote::quote! {[#base_type]}
            }
        };

        match (self.need_params_struct_lifetime(), optional, lifetime) {
            (true, true, Some(lifetime)) => quote::quote! {Option<&#lifetime #wrapped_type>},
            (true, false, Some(lifetime)) => quote::quote! {&#lifetime #wrapped_type},
            (true, true, None) => quote::quote! {Option<&#wrapped_type>},
            (true, false, None) => quote::quote! {&#wrapped_type},
            (false, true, _) => quote::quote! {Option<#wrapped_type>},
            (false, false, _) => wrapped_type,
        }
    }
}

pub trait TypeMapper {
    fn find_rs_type(&self, db_type_name: &str) -> Option<&RsType>;
    fn find_column_type(&self, column: &plugin::Column) -> Option<RsType> {
        let col_type = column.r#type.as_ref().map(make_column_type)?;
        self.find_rs_type(&col_type).cloned()
    }
    fn insert_db_type(&mut self, db_type: &str, rs_type: RsType);
}

#[derive(Default)]
pub(crate) struct SimpleTypeMap {
    /// db_type to rust type
    map: std::collections::BTreeMap<String, RsType>,
}

impl TypeMapper for SimpleTypeMap {
    fn find_rs_type(&self, db_type_name: &str) -> Option<&RsType> {
        self.map.get(db_type_name)
    }

    fn insert_db_type(&mut self, db_type: &str, rs_type: RsType) {
        self.map.insert(db_type.to_string(), rs_type);
    }
}

#[derive(Default)]
pub(crate) struct ColumnTypeMap {
    /// column name to rust type
    column_map: crate::path_map::PathMap<RsType>,
}

impl ColumnTypeMap {
    pub(crate) fn insert(&mut self, column_name: &str, rs_type: RsType) {
        self.column_map.insert(column_name.to_string(), rs_type);
    }

    pub(crate) fn find_type(&self, column_name: &str) -> Option<&RsType> {
        self.column_map.find_best_match(column_name)
    }
}

pub(crate) struct DbTypeMap {
    type_map: Box<dyn TypeMapper>,
    column_map: ColumnTypeMap,
}

impl DbTypeMap {
    pub(crate) fn from_dyn(type_map: Box<dyn TypeMapper>) -> Self {
        Self {
            type_map,
            column_map: Default::default(),
        }
    }
}

impl DbTypeMap {
    #[cfg(test)]
    pub(crate) fn find_rs_type(&self, db_type: &str) -> Option<&RsType> {
        self.type_map.find_rs_type(db_type)
    }

    pub(crate) fn get_column_type(&self, column: &plugin::Column) -> Result<RsType, QueryError> {
        let db_col_name = make_column_name(column);
        if let Some(rs_type) = self.column_map.find_type(&db_col_name) {
            return Ok(rs_type.clone());
        };

        let db_col_type = column
            .r#type
            .as_ref()
            .map(make_column_type)
            .ok_or_else(|| QueryError::missing_column_type(db_col_name.clone()))?
            .to_lowercase();

        self.type_map
            .find_column_type(column)
            .ok_or_else(|| QueryError::cannot_map_type(db_col_type, db_col_name))
    }

    pub(crate) fn insert_db_type(&mut self, db_type: &str, rs_type: RsType) {
        self.type_map.insert_db_type(db_type, rs_type);
    }

    pub(crate) fn insert_column_type(&mut self, column_name: &str, rs_type: RsType) {
        self.column_map.insert(column_name, rs_type);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn override_types_default_only_when_flagged() {
        let column = |can_default| RsColType {
            rs_type: RsType::new(syn::parse_str("crate::Cents").unwrap(), None, true)
                .with_can_default(can_default),
            dim: 0,
            optional: false,
        };

        assert!(!column(false).can_default());
        assert!(column(true).can_default());
    }

    #[test]
    fn params_struct_types_borrow_only_slices() {
        let lifetime = syn::Lifetime::new("'a", proc_macro2::Span::call_site());
        let string = RsColType {
            rs_type: RsType::new(
                syn::parse_str("String").unwrap(),
                Some(syn::parse_str("str").unwrap()),
                false,
            ),
            dim: 0,
            optional: false,
        };
        let integer = RsColType {
            rs_type: RsType::new(syn::parse_str("i64").unwrap(), None, true),
            dim: 0,
            optional: false,
        };
        let override_type = RsColType {
            rs_type: RsType::new(
                syn::parse_str("chrono::DateTime<chrono::Utc>").unwrap(),
                None,
                false,
            ),
            dim: 0,
            optional: false,
        };
        let slice = RsColType {
            rs_type: RsType::new(syn::parse_str("i64").unwrap(), None, true),
            dim: 1,
            optional: false,
        };
        let nullable = RsColType {
            rs_type: RsType::new(syn::parse_str("i64").unwrap(), None, true),
            dim: 0,
            optional: true,
        };

        assert_eq!(
            string.to_params_struct_tokens(Some(&lifetime)).to_string(),
            "& 'a str"
        );
        assert_eq!(
            integer.to_params_struct_tokens(Some(&lifetime)).to_string(),
            "i64"
        );
        assert_eq!(
            override_type
                .to_params_struct_tokens(Some(&lifetime))
                .to_string(),
            "chrono :: DateTime < chrono :: Utc >"
        );
        assert_eq!(
            slice.to_params_struct_tokens(Some(&lifetime)).to_string(),
            "& 'a [i64]"
        );
        assert_eq!(
            nullable
                .to_params_struct_tokens(Some(&lifetime))
                .to_string(),
            "Option < i64 >"
        );
    }
}

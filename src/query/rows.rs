use crate::{field_ident, plugin, value_ident};

use super::{DbTypeMap, QueryError, RsColType, generate_column_names, make_column_name};

#[derive(Clone)]
pub(crate) struct DbEnum {
    /// name of enum
    ///
    /// ```sql
    /// CREATE TYPE book_type AS ENUM (
    ///             ^^^^^^^^^
    ///           'FICTION',
    ///           'NONFICTION'
    /// );
    /// ```
    pub(crate) name: String,

    /// values of enum
    ///
    /// ```sql
    /// CREATE TYPE book_type AS ENUM (
    ///           'FICTION',
    ///            ^^^^^^^
    ///           'NONFICTION'
    ///            ^^^^^^^^^^
    /// );
    /// ```
    pub(crate) values: Vec<String>,

    /// additional derives for enum
    pub(crate) derives: Vec<syn::Path>,
}

impl DbEnum {
    pub(crate) fn ident(&self) -> syn::Ident {
        value_ident(&self.name)
    }
}

pub(crate) fn collect_enums(catalog: &plugin::Catalog) -> Result<Vec<DbEnum>, QueryError> {
    catalog
        .schemas
        .iter()
        .flat_map(|schema| &schema.enums)
        .map(|s_enum| {
            super::validate_unique_members(
                format_args!("Enum `{}`", s_enum.name),
                "variant",
                s_enum
                    .vals
                    .iter()
                    .map(|value| (crate::value_ident(value).to_string(), value.clone())),
            )?;
            Ok(DbEnum {
                name: s_enum.name.clone(),
                values: s_enum.vals.clone(),
                derives: Vec::new(),
            })
        })
        .collect()
}

#[derive(Clone)]
pub(crate) struct ColumnField {
    /// normalized field name
    pub(crate) name: syn::Ident,
    /// original field name
    pub(crate) name_original: syn::LitStr,
    pub(crate) typ: ColumnFieldType,
    pub(crate) attribute: Option<proc_macro2::TokenStream>,
}

impl ColumnField {
    /// `(rust ident, source name)` as `validate_unique_members` wants it.
    pub(crate) fn conflict_pair(&self) -> (String, String) {
        (self.name.to_string(), self.name_original.value())
    }
}

#[derive(Clone)]
pub(crate) enum ColumnFieldType {
    Scalar(Box<RsColType>),
    Embed(EmbeddedTable),
}

impl ColumnFieldType {
    fn to_row_tokens(&self) -> proc_macro2::TokenStream {
        match self {
            Self::Scalar(typ) => typ.to_row_tokens(),
            Self::Embed(table) => {
                let ident = &table.ident;
                quote::quote! {#ident}
            }
        }
    }

    fn width(&self) -> usize {
        match self {
            Self::Scalar(_) => 1,
            Self::Embed(table) => table.fields.len(),
        }
    }
}

#[derive(Clone)]
pub(crate) struct EmbeddedTable {
    pub(crate) qualified_name: String,
    pub(crate) ident: syn::Ident,
    pub(crate) fields: Vec<ColumnField>,
    pub(crate) attributes: Option<proc_macro2::TokenStream>,
}

impl EmbeddedTable {
    fn from_catalog(
        db_type: &DbTypeMap,
        attribute_map: &ReturnRowAttributes,
        table: &plugin::Table,
        identifier: &plugin::Identifier,
    ) -> Result<Self, QueryError> {
        let columns = table
            .columns
            .iter()
            .cloned()
            .map(|mut column| {
                column.table = Some(identifier.clone());
                column
            })
            .collect::<Vec<_>>();
        let field_names = generate_column_names(&columns)
            .into_iter()
            .map(|name| field_ident(&name));
        let attributes = columns
            .iter()
            .zip(field_names.clone())
            .map(|(column, name)| {
                attribute_map
                    .column_attributes
                    .find_best_match(&format!(".{}.{}", identifier.name, name))
                    .or_else(|| {
                        attribute_map
                            .column_attributes
                            .find_best_match(&make_column_name(column))
                    })
                    .cloned()
            });
        let fields = columns
            .iter()
            .zip(field_names)
            .zip(attributes)
            .map(|((column, name), attribute)| {
                Ok(ColumnField {
                    name_original: syn::LitStr::new(&column.name, proc_macro2::Span::call_site()),
                    name,
                    typ: ColumnFieldType::Scalar(Box::new(RsColType::new_with_type(
                        db_type, column,
                    )?)),
                    attribute,
                })
            })
            .collect::<Result<Vec<_>, QueryError>>()?;
        super::validate_unique_members(
            format_args!("Embedded table `{}`", identifier.name),
            "field",
            fields.iter().map(ColumnField::conflict_pair),
        )?;

        Ok(Self {
            qualified_name: if identifier.schema.is_empty() {
                identifier.name.clone()
            } else {
                format!("{}.{}", identifier.schema, identifier.name)
            },
            ident: value_ident(&identifier.name),
            fields,
            attributes: attribute_map
                .row_attributes
                .find_best_match(&format!(".{}", identifier.name))
                .cloned(),
        })
    }
}

impl ColumnField {
    pub(crate) fn scalar_type(&self) -> &RsColType {
        match &self.typ {
            ColumnFieldType::Scalar(typ) => typ,
            ColumnFieldType::Embed(_) => {
                unreachable!("ColumnField::scalar_type parameters are never sqlc.embed columns")
            }
        }
    }

    pub(super) fn scalar_type_mut(&mut self) -> &mut RsColType {
        match &mut self.typ {
            ColumnFieldType::Scalar(typ) => typ,
            ColumnFieldType::Embed(_) => {
                unreachable!("ColumnField::scalar_type parameters are never sqlc.embed columns")
            }
        }
    }

    pub(crate) fn row_type(&self) -> proc_macro2::TokenStream {
        self.typ.to_row_tokens()
    }

    pub(crate) fn embedded_table(&self) -> Option<&EmbeddedTable> {
        match &self.typ {
            ColumnFieldType::Scalar(_) => None,
            ColumnFieldType::Embed(table) => Some(table),
        }
    }

    pub(crate) fn width(&self) -> usize {
        self.typ.width()
    }
}

pub(crate) fn find_embedded_table<'a>(
    catalog: &'a plugin::Catalog,
    identifier: &plugin::Identifier,
) -> Result<&'a plugin::Table, QueryError> {
    catalog
        .schemas
        .iter()
        .flat_map(|schema| schema.tables.iter())
        .find(|table| match &table.rel {
            Some(rel) => {
                rel.name == identifier.name
                    && (identifier.schema.is_empty() || rel.schema == identifier.schema)
            }
            None => false,
        })
        .ok_or_else(|| QueryError::missing_embedded_table(identifier))
}

fn deserialize_path_map<'de, D>(
    deserializer: D,
) -> Result<crate::path_map::PathMap<proc_macro2::TokenStream>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;

    #[derive(Debug, serde::Deserialize)]
    #[serde(untagged)]
    enum SingleOrMany {
        Single(String),
        Many(Vec<String>),
    }

    impl SingleOrMany {
        fn into_token(self) -> Result<proc_macro2::TokenStream, syn::Error> {
            let s = match self {
                SingleOrMany::Single(v) => v,
                SingleOrMany::Many(items) => items.join("\n"),
            };

            syn::parse_str::<proc_macro2::TokenStream>(&s)
        }
    }

    let m = std::collections::BTreeMap::<String, SingleOrMany>::deserialize(deserializer)?;
    let mut map = crate::path_map::PathMap::default();
    for (k, v) in m.into_iter() {
        let v = v.into_token().map_err(serde::de::Error::custom)?;
        map.insert(k, v);
    }
    Ok(map)
}

#[derive(Default, Debug, serde::Deserialize)]
#[serde(default)]
pub(crate) struct ReturnRowAttributes {
    #[serde(deserialize_with = "deserialize_path_map")]
    row_attributes: crate::path_map::PathMap<proc_macro2::TokenStream>,
    #[serde(deserialize_with = "deserialize_path_map")]
    column_attributes: crate::path_map::PathMap<proc_macro2::TokenStream>,
}

#[derive(Clone)]
pub(crate) struct ReturningRows {
    pub(crate) fields: Vec<ColumnField>,
    pub(crate) query_name: String,
    pub(crate) attributes: Option<proc_macro2::TokenStream>,
}

impl ReturningRows {
    pub(crate) fn from_query(
        db_type: &DbTypeMap,
        attribute_map: &ReturnRowAttributes,
        catalog: Option<&plugin::Catalog>,
        query: &plugin::Query,
    ) -> Result<Self, QueryError> {
        let field_names = generate_column_names(&query.columns)
            .into_iter()
            .map(|s| field_ident(&s));
        let original_names = query
            .columns
            .iter()
            .map(|col| syn::LitStr::new(&col.name, proc_macro2::Span::call_site()));
        let column_names = field_names.zip(original_names).collect::<Vec<_>>();

        let column_attributes = query
            .columns
            .iter()
            .zip(column_names.iter())
            .map(|(col, (name, _))| {
                let query_att = attribute_map
                    .column_attributes
                    .find_best_match(&format!(".{}.{}", query.name, name));
                let table_att = attribute_map
                    .column_attributes
                    .find_best_match(&make_column_name(col));
                query_att.or(table_att)
            })
            .collect::<Vec<_>>();

        let fields = column_names
            .into_iter()
            .zip(column_attributes)
            .zip(query.columns.iter())
            .map(|(((col_name, col_name_original), col_attribute), column)| {
                let typ = match &column.embed_table {
                    Some(identifier) => {
                        let catalog = catalog
                            .ok_or_else(|| QueryError::missing_embedded_table(identifier))?;
                        let table = find_embedded_table(catalog, identifier)?;
                        ColumnFieldType::Embed(EmbeddedTable::from_catalog(
                            db_type,
                            attribute_map,
                            table,
                            identifier,
                        )?)
                    }
                    None => ColumnFieldType::Scalar(Box::new(RsColType::new_with_type(
                        db_type, column,
                    )?)),
                };

                Ok(ColumnField {
                    name: col_name,
                    name_original: col_name_original,
                    typ,
                    attribute: col_attribute.cloned(),
                })
            })
            .collect::<Result<Vec<_>, QueryError>>()?;
        super::validate_unique_members(
            format_args!("Query `{}`", query.name),
            "row field",
            fields.iter().map(ColumnField::conflict_pair),
        )?;

        let row_attributes = attribute_map
            .row_attributes
            .find_best_match(&format!(".{}", query.name));

        Ok(Self {
            fields,
            query_name: query.name.to_string(),
            attributes: row_attributes.cloned(),
        })
    }

    pub(crate) fn struct_ident(&self) -> syn::Ident {
        value_ident(&format!("{}Row", self.query_name))
    }

    pub(crate) fn embedded_tables(&self) -> impl Iterator<Item = &EmbeddedTable> {
        self.fields.iter().filter_map(ColumnField::embedded_table)
    }

    pub(crate) fn field_ordinals(&self) -> impl Iterator<Item = std::ops::Range<usize>> + '_ {
        self.fields.iter().scan(0, |ordinal, field| {
            let range = *ordinal..*ordinal + field.width();
            *ordinal += field.width();
            Some(range)
        })
    }
}

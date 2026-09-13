use crate::plugin;
use crate::query::{DbTypeMap, Query, ReturnRowAttributes, ReturningRows};

pub(crate) fn identifier(name: &str) -> plugin::Identifier {
    plugin::Identifier {
        name: name.to_string(),
        schema: String::new(),
        catalog: String::new(),
    }
}

pub(crate) fn column(name: &str, sqlc_slice: bool) -> plugin::Column {
    plugin::Column {
        name: name.to_string(),
        table: None,
        not_null: true,
        is_array: false,
        comment: String::new(),
        length: 0,
        is_named_param: false,
        is_func_call: false,
        scope: String::new(),
        table_alias: String::new(),
        r#type: Some(identifier("integer")),
        is_sqlc_slice: sqlc_slice,
        embed_table: None,
        original_name: String::new(),
        unsigned: false,
        array_dims: 0,
    }
}

pub(crate) fn query(
    name: &str,
    cmd: &str,
    text: &str,
    columns: Vec<plugin::Column>,
    params: Vec<(i32, plugin::Column)>,
) -> plugin::Query {
    plugin::Query {
        text: text.to_string(),
        name: name.to_string(),
        cmd: cmd.to_string(),
        columns,
        params: params
            .into_iter()
            .map(|(number, column)| plugin::Parameter {
                number,
                column: Some(column),
            })
            .collect(),
        comments: Vec::new(),
        filename: String::new(),
        insert_into_table: None,
    }
}

pub(crate) fn parse_query(
    type_map: &DbTypeMap,
    catalog: Option<&plugin::Catalog>,
    plugin_query: &plugin::Query,
) -> (ReturningRows, Query) {
    let row = ReturningRows::from_query(
        type_map,
        &ReturnRowAttributes::default(),
        catalog,
        plugin_query,
    )
    .unwrap();
    let mut query = Query::from_query(type_map, plugin_query).unwrap();
    query.apply_dynfilter();
    (row, query)
}

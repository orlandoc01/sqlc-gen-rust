use crate::{
    db_crates::{rusqlite::Rusqlite, rusqlite_params},
    plugin,
    query::{Query, ReturnRowAttributes, ReturningRows},
};

fn identifier(name: &str) -> plugin::Identifier {
    plugin::Identifier {
        name: name.to_string(),
        schema: String::new(),
        catalog: String::new(),
    }
}

fn column(name: &str, sqlc_slice: bool) -> plugin::Column {
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

fn query(
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

fn generated(query: plugin::Query, query_parameter_limit: usize) -> String {
    let type_map = Rusqlite.db_type_map();
    let row = ReturningRows::from_query(&type_map, &ReturnRowAttributes::default(), None, &query)
        .unwrap();
    let mut query = Query::from_query(&type_map, &query).unwrap();
    query.apply_dynfilter();
    if query.dynfilter().is_none() {
        query.apply_static_slices();
    }
    rusqlite_params::generate_queries(&[row], &[query], query_parameter_limit).to_string()
}

#[test]
fn generates_one_with_a_params_struct() {
    let tokens = generated(
        query(
            "GetAuthor",
            ":one",
            "SELECT id FROM authors WHERE id = ? AND owner_id = ?",
            vec![column("id", false)],
            vec![(1, column("id", false)), (2, column("owner_id", false))],
        ),
        1,
    );

    assert!(tokens.contains("pub struct GetAuthorParams"));
    assert!(
        tokens.contains(
            "pub fn get_author (client : & impl RusqliteClient , params : GetAuthorParams"
        )
    );
    assert!(tokens.contains("client . connection () . prepare_cached (GET_AUTHOR)"));
    assert!(tokens.contains("statement . query_row (params , GetAuthorRow :: from_row)"));
}

#[test]
fn generates_many_with_a_slice() {
    let tokens = generated(
        query(
            "ListAuthors",
            ":many",
            "SELECT id FROM authors WHERE id IN (/*SLICE:ids*/?1)",
            vec![column("id", false)],
            vec![(1, column("ids", true))],
        ),
        1,
    );

    assert!(
        tokens.contains("dynfilter :: Placeholders :: NumberedSqlite"),
        "{tokens}"
    );
    assert!(tokens.contains("client . connection () . prepare (& sql)"));
    assert!(tokens.contains("dynfilter :: Bind :: Elem (0usize , element)"));
    assert!(tokens.contains("statement . query_map (params , ListAuthorsRow :: from_row)"));
}

#[test]
fn generates_exec_last_id() {
    let tokens = generated(
        query(
            "CreateAuthor",
            ":execlastid",
            "INSERT INTO authors DEFAULT VALUES",
            Vec::new(),
            Vec::new(),
        ),
        1,
    );

    assert!(tokens.contains(
        "pub fn create_author (client : & impl RusqliteClient) -> rusqlite :: Result < i64 >"
    ));
    assert!(tokens.contains(
        "let mut rows = statement . query (params) ? ; while rows . next () ? . is_some () { } Ok (client . connection () . last_insert_rowid ())"
    ));
}

#[test]
fn generates_dynamic_filters() {
    let tokens = generated(
        query(
            "SearchAuthors",
            ":many",
            "SELECT id FROM authors WHERE TRUE\nAND id = ? -- :if @id",
            vec![column("id", false)],
            vec![(1, column("id", false))],
        ),
        1,
    );

    assert!(tokens.contains("static SEARCH_AUTHORS_DYN"));
    assert!(tokens.contains("dynfilter :: Arg :: from_option (& params . id)"));
    assert!(tokens.contains("map (| bind | -> & dyn rusqlite :: ToSql"));
    assert!(tokens.contains("rusqlite :: params_from_iter (values)"));
}

#[test]
fn direct_parameters_never_collide_with_generated_locals() {
    let tokens = generated(
        query(
            "ByStatement",
            ":many",
            "SELECT id FROM authors WHERE statement = ?1 AND client = ?2 AND params = ?3",
            vec![column("id", false)],
            vec![
                (1, column("statement", false)),
                (2, column("client", false)),
                (3, column("params", false)),
            ],
        ),
        3,
    );

    assert!(
        tokens.contains(
            "pub fn by_statement (client_ : & impl RusqliteClient , statement : i64 , client : i64 , params : i64)"
        ),
        "{tokens}"
    );
    assert!(tokens.contains("let params_ = rusqlite :: params ! [statement , client , params] ;"));
    assert!(tokens.contains(
        "let mut statement_ = client_ . connection () . prepare_cached (BY_STATEMENT) ? ;"
    ));
    assert!(tokens.contains("statement_ . query_map (params_ , ByStatementRow :: from_row)"));
}

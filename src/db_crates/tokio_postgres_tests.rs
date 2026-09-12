use crate::{
    db_crates::{
        TokioPostgres, params_common,
        test_support::{column, query},
    },
    plugin,
    query::{self, EmbeddedTable, Query, ReturnRowAttributes, ReturningRows},
};

fn generated(query: plugin::Query, query_parameter_limit: usize) -> String {
    let type_map = TokioPostgres.db_type_map();
    let row = ReturningRows::from_query(&type_map, &ReturnRowAttributes::default(), None, &query)
        .unwrap();
    let mut query = Query::from_query(&type_map, &query).unwrap();
    query.apply_dynfilter();
    params_common::generate_queries(&TokioPostgres, &[row], &[query], query_parameter_limit)
        .to_string()
}

#[test]
fn generates_static_statement_functions_and_parameter_array() {
    let tokens = generated(
        query(
            "GetAuthor",
            ":one",
            "SELECT id FROM authors WHERE id = $1",
            vec![column("id", false)],
            vec![(1, column("id", false))],
        ),
        1,
    );

    assert!(tokens.contains("prepare_get_author"));
    assert!(
        tokens.contains(
            "get_author_with < S : ? Sized + tokio_postgres :: ToStatement + Sync + Send >"
        )
    );
    assert!(tokens.contains("& [& id ,]"));
}

#[test]
fn generates_dynamic_functions_without_statements() {
    let tokens = generated(
        query(
            "SearchAuthors",
            ":many",
            "SELECT id FROM authors WHERE TRUE\nAND id = $1 -- :if @id",
            vec![column("id", false)],
            vec![(1, column("id", false))],
        ),
        1,
    );

    assert!(tokens.contains("fn search_authors_query < 'p >"));
    assert!(tokens.contains("dynfilter :: Bind :: Arg (0usize) => & params . id as _"));
    assert!(!tokens.contains("prepare_search_authors"));
    assert!(tokens.contains("fn search_authors_stream"));
    assert!(tokens.contains("Result < Vec < SearchAuthorsRow >"));
    assert!(tokens.contains("params : & 'p SearchAuthorsParams"));
    assert_eq!(tokens.matches("search_authors_query (& params)").count(), 2);
}

#[test]
fn direct_parameters_do_not_collide_with_generated_locals() {
    let tokens = generated(
        query(
            "ByStatement",
            ":many",
            "SELECT id FROM authors WHERE statement = $1 AND client = $2 AND values = $3",
            vec![column("id", false)],
            vec![
                (1, column("statement", false)),
                (2, column("client", false)),
                (3, column("values", false)),
            ],
        ),
        3,
    );

    assert!(tokens.contains(
        "pub async fn by_statement (client_ : & impl tokio_postgres :: GenericClient , statement : i32 , client : i32 , values : i32)"
    ));
    assert!(tokens.contains("statement_ : & S"));
    assert!(tokens.contains("let values_ : & [& (dyn tokio_postgres :: types :: ToSql + Sync)]"));
}

#[test]
fn embedded_rows_decode_by_select_ordinal() {
    let type_map = TokioPostgres.db_type_map();
    let scalar = |name| query::ColumnField {
        name: crate::field_ident(name),
        name_original: syn::LitStr::new(name, proc_macro2::Span::call_site()),
        typ: query::ColumnFieldType::Scalar(Box::new(
            query::RsColType::new_with_type(&type_map, &column(name, false)).unwrap(),
        )),
        attribute: None,
    };
    let row = ReturningRows {
        fields: vec![
            scalar("before"),
            query::ColumnField {
                name: crate::field_ident("authors"),
                name_original: syn::LitStr::new("authors", proc_macro2::Span::call_site()),
                typ: query::ColumnFieldType::Embed(EmbeddedTable {
                    qualified_name: "authors".to_string(),
                    ident: crate::value_ident("authors"),
                    fields: vec![scalar("id"), scalar("name")],
                    attributes: None,
                }),
                attribute: None,
            },
        ],
        query_name: "GetAuthors".to_string(),
        attributes: None,
    };

    let tokens = TokioPostgres.returning_ordinal_row(&row).to_string();
    assert!(tokens.contains("before : row . try_get (0) ?"));
    assert!(tokens.contains("id : row . try_get (1) ?"));
    assert!(tokens.contains("name : row . try_get (2) ?"));
}

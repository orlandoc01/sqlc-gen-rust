use crate::{
    db_crates::{
        TokioPostgres, params_common,
        test_support::{column, identifier, query},
    },
    plugin,
    query::{self, EmbeddedTable, Query, ReturnRowAttributes, ReturningRows},
};

fn generated_tokens(
    backend: TokioPostgres,
    query: plugin::Query,
    query_parameter_limit: usize,
) -> proc_macro2::TokenStream {
    let type_map = backend.db_type_map();
    let row = ReturningRows::from_query(&type_map, &ReturnRowAttributes::default(), None, &query)
        .unwrap();
    let mut query = Query::from_query(&type_map, &query).unwrap();
    query.apply_dynfilter();
    params_common::generate_queries(&backend, &[row], &[query], query_parameter_limit).unwrap()
}

fn generated(backend: TokioPostgres, query: plugin::Query, query_parameter_limit: usize) -> String {
    generated_tokens(backend, query, query_parameter_limit).to_string()
}

fn static_query() -> plugin::Query {
    query(
        "GetAuthor",
        ":one",
        "SELECT id FROM authors WHERE id = $1",
        vec![column("id", false)],
        vec![(1, column("id", false))],
    )
}

#[test]
fn generates_static_statement_functions_and_parameter_array() {
    for backend in [TokioPostgres::Tokio, TokioPostgres::Deadpool] {
        let tokens = generated(backend, static_query(), 1);
        assert!(tokens.contains("prepare_get_author"));
        assert!(tokens.contains("& [& id ,]"));
    }

    let tokens = generated(TokioPostgres::Tokio, static_query(), 1);
    assert!(
        tokens.contains(
            "get_author_with < S : ? Sized + tokio_postgres :: ToStatement + Sync + Send >"
        )
    );
}

#[test]
fn generates_dynamic_functions_without_statements() {
    for backend in [TokioPostgres::Tokio, TokioPostgres::Deadpool] {
        let tokens = generated(
            backend,
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
}

#[test]
fn direct_parameters_do_not_collide_with_generated_locals() {
    let tokens = generated(
        TokioPostgres::Tokio,
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
fn direct_parameters_do_not_shadow_forwarding_functions() {
    let tokens = generated_tokens(
        TokioPostgres::Tokio,
        query(
            "GetAuthor",
            ":one",
            "SELECT id FROM authors WHERE id = $1",
            vec![column("id", false)],
            vec![(1, column("get_author_with", false))],
        ),
        1,
    );
    assert!(syn::parse2::<syn::File>(tokens.clone()).is_ok());
    let tokens = tokens.to_string();

    assert!(tokens.contains("self :: get_author_with (client , GET_AUTHOR , get_author_with)"));

    let opt_tokens = generated(
        TokioPostgres::Tokio,
        query(
            "GetAuthor",
            ":one",
            "SELECT id FROM authors WHERE id = $1",
            vec![column("id", false)],
            vec![(1, column("get_author_opt_with", false))],
        ),
        1,
    );
    assert!(
        opt_tokens
            .contains("self :: get_author_opt_with (client , GET_AUTHOR , get_author_opt_with)")
    );

    let stream_tokens = generated(
        TokioPostgres::Tokio,
        query(
            "ListAuthors",
            ":many",
            "SELECT id FROM authors WHERE id = $1",
            vec![column("id", false)],
            vec![(1, column("list_authors_stream_with", false))],
        ),
        1,
    );
    assert!(stream_tokens.contains(
        "self :: list_authors_stream_with (client , LIST_AUTHORS , list_authors_stream_with)"
    ));
}

#[test]
fn required_system_time_params_do_not_derive_default() {
    let mut timestamp = column("timestamp", false);
    timestamp.r#type = Some(identifier("timestamp"));
    let tokens = generated(
        TokioPostgres::Tokio,
        query(
            "EchoTimestamp",
            ":one",
            "SELECT $1::timestamp AS timestamp",
            vec![timestamp.clone()],
            vec![(1, timestamp)],
        ),
        0,
    );

    assert!(tokens.contains("pub struct EchoTimestampParams"));
    assert!(tokens.contains("std :: time :: SystemTime"));
    assert!(!tokens.contains("derive (Debug , Clone , Default)"));
}

#[test]
fn uses_deadpool_paths_and_cached_statements() {
    let deadpool = generated(TokioPostgres::Deadpool, static_query(), 1);
    assert!(deadpool.contains("prepare_cached"));
    assert!(deadpool.contains("deadpool_postgres :: GenericClient"));
    assert!(deadpool.contains("deadpool_postgres :: tokio_postgres :: types :: ToSql"));

    let tokio = generated(TokioPostgres::Tokio, static_query(), 1);
    assert!(!tokio.contains("prepare_cached"));
}

#[test]
fn deadpool_to_sql_path() {
    assert_eq!(
        TokioPostgres::Deadpool.paths().to_sql.to_string(),
        "deadpool_postgres :: tokio_postgres :: types :: ToSql"
    );
}

#[test]
fn embedded_rows_decode_by_select_ordinal() {
    let type_map = TokioPostgres::Tokio.db_type_map();
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

    for backend in [TokioPostgres::Tokio, TokioPostgres::Deadpool] {
        let tokens = backend.returning_ordinal_row(&row).to_string();
        assert!(tokens.contains("before : row . try_get (0) ?"));
        assert!(tokens.contains("id : row . try_get (1) ?"));
        assert!(tokens.contains("name : row . try_get (2) ?"));
    }
}

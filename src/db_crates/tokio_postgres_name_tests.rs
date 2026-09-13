use crate::{
    db_crates::{
        TokioPostgres, params_common,
        sqlx::Sqlx,
        test_support::{column, query},
    },
    plugin,
    query::{self, Query, ReturnRowAttributes, ReturningRows},
};

fn parsed_queries(
    type_map: &query::DbTypeMap,
    queries: Vec<plugin::Query>,
) -> (Vec<ReturningRows>, Vec<Query>) {
    queries
        .into_iter()
        .map(|query| {
            let row =
                ReturningRows::from_query(type_map, &ReturnRowAttributes::default(), None, &query)
                    .unwrap();
            let mut query = Query::from_query(type_map, &query).unwrap();
            query.apply_dynfilter();
            (row, query)
        })
        .unzip()
}

fn collision_error(first: plugin::Query, second: plugin::Query) -> String {
    let type_map = TokioPostgres::Tokio.db_type_map();
    let (rows, queries) = parsed_queries(&type_map, vec![first, second]);
    params_common::generate_queries(&TokioPostgres::Tokio, &rows, &queries, 1)
        .unwrap_err()
        .to_string()
}

#[test]
fn rejects_prepare_function_name_collisions() {
    let error = collision_error(
        query(
            "GetAuthor",
            ":one",
            "SELECT id FROM authors",
            vec![column("id", false)],
            vec![],
        ),
        query(
            "PrepareGetAuthor",
            ":one",
            "SELECT id FROM authors",
            vec![column("id", false)],
            vec![],
        ),
    );

    assert_eq!(
        error,
        "Queries `GetAuthor` (prepare helper) and `PrepareGetAuthor` (query function) both generate Rust function `prepare_get_author`"
    );
}

#[test]
fn rejects_with_function_name_collisions() {
    let error = collision_error(
        query(
            "GetAuthor",
            ":one",
            "SELECT id FROM authors",
            vec![column("id", false)],
            vec![],
        ),
        query(
            "GetAuthorWith",
            ":exec",
            "DELETE FROM authors",
            Vec::new(),
            vec![],
        ),
    );

    assert_eq!(
        error,
        "Queries `GetAuthor` (with helper) and `GetAuthorWith` (query function) both generate Rust function `get_author_with`"
    );
}

#[test]
fn rejects_stream_function_name_collisions() {
    let error = collision_error(
        query(
            "ListAuthors",
            ":many",
            "SELECT id FROM authors",
            vec![column("id", false)],
            vec![],
        ),
        query(
            "ListAuthorsStream",
            ":exec",
            "DELETE FROM authors",
            Vec::new(),
            vec![],
        ),
    );

    assert_eq!(
        error,
        "Queries `ListAuthors` (stream helper) and `ListAuthorsStream` (query function) both generate Rust function `list_authors_stream`"
    );
}

#[test]
fn rejects_dynamic_helper_name_collisions() {
    let error = collision_error(
        query(
            "SearchAuthors",
            ":many",
            "SELECT id FROM authors WHERE id = $1 -- :if @id",
            vec![column("id", false)],
            vec![(1, column("id", false))],
        ),
        query(
            "SearchAuthorsQuery",
            ":exec",
            "DELETE FROM authors",
            Vec::new(),
            vec![],
        ),
    );

    assert_eq!(
        error,
        "Queries `SearchAuthors` (dynamic query helper) and `SearchAuthorsQuery` (query function) both generate Rust function `search_authors_query`"
    );
}

#[test]
fn does_not_reserve_tokio_helpers_for_sqlx() {
    let type_map = Sqlx::Sqlite.db_type_map();
    let (rows, queries) = parsed_queries(
        &type_map,
        vec![
            query(
                "GetAuthor",
                ":one",
                "SELECT id FROM authors",
                vec![column("id", false)],
                vec![],
            ),
            query(
                "PrepareGetAuthor",
                ":one",
                "SELECT id FROM authors",
                vec![column("id", false)],
                vec![],
            ),
        ],
    );

    assert!(params_common::generate_queries(&Sqlx::Sqlite, &rows, &queries, 1).is_ok());
}

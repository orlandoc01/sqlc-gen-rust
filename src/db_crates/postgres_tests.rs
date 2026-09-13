use crate::{
    db_crates::{
        Postgres, params_common,
        postgres_params::PostgresParams,
        test_support::{column, identifier, query},
    },
    plugin,
    query::{self, EmbeddedTable, Query, ReturnRowAttributes, ReturningRows},
};

fn generated_tokens(
    backend: Postgres,
    query: plugin::Query,
    query_parameter_limit: usize,
) -> proc_macro2::TokenStream {
    let type_map = backend.db_type_map();
    generated_tokens_with_type_map(backend, &type_map, query, query_parameter_limit)
}

fn generated_tokens_with_type_map(
    backend: Postgres,
    type_map: &query::DbTypeMap,
    query: plugin::Query,
    query_parameter_limit: usize,
) -> proc_macro2::TokenStream {
    let row =
        ReturningRows::from_query(type_map, &ReturnRowAttributes::default(), None, &query).unwrap();
    let mut query = Query::from_query(type_map, &query).unwrap();
    query.apply_dynfilter();
    params_common::generate_queries(&backend, &[row], &[query], query_parameter_limit).unwrap()
}

fn generated(backend: Postgres, query: plugin::Query, query_parameter_limit: usize) -> String {
    generated_tokens(backend, query, query_parameter_limit).to_string()
}

fn generated_typed(
    backend: Postgres,
    query: plugin::Query,
    query_parameter_limit: usize,
) -> String {
    let type_map = backend.db_type_map();
    let row = ReturningRows::from_query(&type_map, &ReturnRowAttributes::default(), None, &query)
        .unwrap();
    let mut query = Query::from_query(&type_map, &query).unwrap();
    query.apply_dynfilter();
    params_common::generate_queries(
        &PostgresParams {
            backend,
            query_typed: true,
        },
        &[row],
        &[query],
        query_parameter_limit,
    )
    .unwrap()
    .to_string()
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

fn echo_timestamp_query() -> plugin::Query {
    let mut timestamp = column("timestamp", false);
    timestamp.r#type = Some(identifier("timestamp"));
    query(
        "EchoTimestamp",
        ":one",
        "SELECT $1::timestamp AS timestamp",
        vec![timestamp.clone()],
        vec![(1, timestamp)],
    )
}

#[test]
fn generates_static_statement_functions_and_parameter_array() {
    for backend in [Postgres::Sync, Postgres::Tokio, Postgres::Deadpool] {
        let tokens = generated(backend, static_query(), 1);
        assert!(tokens.contains("prepare_get_author"));
        assert!(tokens.contains("& [& id ,]"));
    }

    let tokens = generated(Postgres::Tokio, static_query(), 1);
    assert!(
        tokens.contains(
            "get_author_with (client : & impl tokio_postgres :: GenericClient , statement : & (impl tokio_postgres :: ToStatement + ? :: std :: marker :: Sized + :: std :: marker :: Sync + :: std :: marker :: Send)"
        )
    );
    for backend in [Postgres::Sync, Postgres::Tokio] {
        let tokens = generated(backend, static_query(), 1);
        assert!(tokens.contains(":: std :: marker :: Sync"));
        assert!(!tokens.contains("+ Sync"));
    }
}

#[test]
fn generates_typed_static_queries_and_keeps_prepared_helpers() {
    for backend in [Postgres::Sync, Postgres::Tokio, Postgres::Deadpool] {
        for (annotation, method) in [
            (":one", "query_typed_one"),
            (":many", "query_typed ("),
            (":exec", "query_typed_raw"),
            (":execrows", "query_typed_raw"),
        ] {
            let tokens = generated_typed(
                backend,
                query(
                    "TypedQuery",
                    annotation,
                    "SELECT id FROM authors WHERE id = $1",
                    vec![column("id", false)],
                    vec![(1, column("id", false))],
                ),
                1,
            );
            assert!(tokens.contains(method));
            assert!(tokens.contains("Type :: INT4"));
            assert!(tokens.contains("prepare_typed_query"));
            assert!(tokens.contains("typed_query_with"));
        }
    }
}

#[test]
fn maps_typed_arrays_and_catalog_spellings() {
    for backend in [Postgres::Sync, Postgres::Tokio, Postgres::Deadpool] {
        let mut ids = column("ids", false);
        ids.array_dims = 1;
        let mut catalog_id = column("catalog_id", false);
        catalog_id.r#type = Some(plugin::Identifier {
            name: "int4".to_string(),
            schema: "pg_catalog".to_string(),
            catalog: String::new(),
        });
        let tokens = generated_typed(
            backend,
            query(
                "TypedArrays",
                ":one",
                "SELECT id FROM authors WHERE id = ANY($1) AND id = $2",
                vec![column("id", false)],
                vec![(1, ids), (2, catalog_id)],
            ),
            1,
        );
        assert!(tokens.contains("Type :: INT4_ARRAY"));
        assert!(tokens.contains("Type :: INT4"));
    }
}

#[test]
fn falls_back_for_unknown_typed_parameters() {
    for backend in [Postgres::Sync, Postgres::Tokio, Postgres::Deadpool] {
        let mut type_map = backend.db_type_map();
        type_map.insert_db_type(
            "state",
            query::RsType::new(syn::parse_str("crate::State").unwrap(), None, true),
        );
        let enum_column = plugin::Column {
            r#type: Some(identifier("state")),
            ..column("state", false)
        };
        let row = ReturningRows::from_query(
            &type_map,
            &ReturnRowAttributes::default(),
            None,
            &query(
                "ByState",
                ":one",
                "SELECT id FROM authors WHERE state = $1",
                vec![column("id", false)],
                vec![(1, enum_column.clone())],
            ),
        )
        .unwrap();
        let query = Query::from_query(
            &type_map,
            &query(
                "ByState",
                ":one",
                "SELECT id FROM authors WHERE state = $1",
                vec![column("id", false)],
                vec![(1, enum_column)],
            ),
        )
        .unwrap();
        let tokens = params_common::generate_queries(
            &PostgresParams {
                backend,
                query_typed: true,
            },
            &[row],
            &[query],
            1,
        )
        .unwrap()
        .to_string();
        assert!(tokens.contains("query_one ("));
        assert!(!tokens.contains("query_typed"));
    }
}

#[test]
fn generates_typed_dynamic_queries_and_leaves_the_default_off() {
    for backend in [Postgres::Sync, Postgres::Tokio, Postgres::Deadpool] {
        let query = query(
            "SearchAuthors",
            ":many",
            "SELECT id FROM authors WHERE TRUE\nAND id = $1 -- :if @id",
            vec![column("id", false)],
            vec![(1, column("id", false))],
        );
        let typed = generated_typed(backend, query.clone(), 1);
        assert!(typed.contains("Vec < (& (dyn"));
        assert!(typed.contains("Type :: INT4"));
        assert!(typed.contains("query_typed (sql . as_str ()"));
        assert!(typed.contains("query_typed_raw (sql . as_str ()"));
        assert!(!typed.contains("prepare_search_authors"));

        assert!(!generated(backend, query, 1).contains("query_typed"));
    }
}

#[test]
fn generates_dynamic_functions_without_statements() {
    for backend in [Postgres::Sync, Postgres::Tokio, Postgres::Deadpool] {
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

        assert!(tokens.contains("fn search_authors_query (params"));
        assert!(tokens.contains("dynfilter :: Bind :: Arg (0usize) => & params . id as _"));
        assert!(!tokens.contains("prepare_search_authors"));
        assert!(tokens.contains("Result < Vec < SearchAuthorsRow >"));
        assert!(tokens.contains("params : & SearchAuthorsParams"));
        assert_eq!(tokens.matches("search_authors_query (& params)").count(), 2);
    }
}

#[test]
fn sync_uses_mutable_clients_and_row_iterators() {
    let query = query(
        "ListAuthors",
        ":many",
        "SELECT id FROM authors WHERE id = $1",
        vec![column("id", false)],
        vec![(1, column("id", false))],
    );
    let tokens = generated_tokens(Postgres::Sync, query, 1);
    assert!(syn::parse2::<syn::File>(tokens.clone()).is_ok());
    let tokens = tokens.to_string();

    assert!(tokens.contains("client : & mut impl postgres :: GenericClient"));
    assert!(tokens.contains("prepare_list_authors"));
    assert!(tokens.contains("list_authors_iter"));
    assert!(tokens.contains("list_authors_iter_with"));
    assert!(tokens.contains("self :: list_authors_iter_with (client , LIST_AUTHORS , id)"));
    assert!(!tokens.contains("list_authors_stream"));
    assert!(tokens.contains("Result < postgres :: RowIter < 'c > , postgres :: Error >"));
    assert!(tokens.contains("query_raw (statement , values . iter () . copied ())"));
    assert!(!tokens.contains("async fn"));
    assert!(!tokens.contains(". await"));
}

#[test]
fn sync_dynamic_many_uses_iter_and_the_dynamic_helper() {
    let tokens = generated_tokens(
        Postgres::Sync,
        query(
            "SearchAuthors",
            ":many",
            "SELECT id FROM authors WHERE TRUE\nAND id = $1 -- :if @id",
            vec![column("id", false)],
            vec![(1, column("id", false))],
        ),
        1,
    );
    assert!(syn::parse2::<syn::File>(tokens.clone()).is_ok());
    let tokens = tokens.to_string();

    assert!(tokens.contains("fn search_authors_query (params"));
    assert!(tokens.contains("fn search_authors_iter"));
    assert!(!tokens.contains("search_authors_stream"));
    assert!(tokens.contains("Result < postgres :: RowIter < 'c > , postgres :: Error >"));
    assert!(!tokens.contains("async fn"));
    assert!(!tokens.contains(". await"));
}

#[test]
fn tokio_uses_async_stream_functions() {
    let tokens = generated(
        Postgres::Tokio,
        query(
            "ListAuthors",
            ":many",
            "SELECT id FROM authors",
            vec![column("id", false)],
            Vec::new(),
        ),
        1,
    );

    assert!(tokens.contains("pub async fn"));
    assert!(tokens.contains("list_authors_stream"));
    assert!(!tokens.contains("list_authors_iter"));
}

#[test]
fn direct_parameters_do_not_collide_with_generated_locals() {
    for backend in [Postgres::Sync, Postgres::Tokio, Postgres::Deadpool] {
        let tokens = generated(
            backend,
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
        let paths = backend.paths();

        assert!(tokens.contains(&format!(
            "client_ : {} impl {} , statement : i32 , client : i32 , values : i32",
            paths.plain.client_ref, paths.client
        )));
        assert!(tokens.contains(&format!(
            "statement_ : & (impl {} + ? :: std :: marker :: Sized + :: std :: marker :: Sync + :: std :: marker :: Send)",
            paths.to_statement
        )));
        assert!(tokens.contains(&format!(
            "let values_ : & [& (dyn {} + :: std :: marker :: Sync)]",
            paths.to_sql
        )));
        assert!(tokens.contains(&format!(
            "self :: by_statement_{}_with",
            backend.many_iterator_suffix()
        )));
    }
}

#[test]
fn direct_parameters_do_not_shadow_forwarding_functions() {
    for backend in [Postgres::Sync, Postgres::Tokio, Postgres::Deadpool] {
        let tokens = generated_tokens(
            backend,
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
        assert!(
            tokens
                .to_string()
                .contains("self :: get_author_with (client , GET_AUTHOR , get_author_with)")
        );

        let opt_tokens = generated(
            backend,
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
            opt_tokens.contains(
                "self :: get_author_opt_with (client , GET_AUTHOR , get_author_opt_with)"
            )
        );

        let suffix = backend.many_iterator_suffix();
        let helper = format!("list_authors_{suffix}_with");
        let iterator_tokens = generated(
            backend,
            query(
                "ListAuthors",
                ":many",
                "SELECT id FROM authors WHERE id = $1",
                vec![column("id", false)],
                vec![(1, column(&helper, false))],
            ),
            1,
        );
        assert!(iterator_tokens.contains(&format!(
            "self :: list_authors_{suffix}_with (client , LIST_AUTHORS , {helper})"
        )));
    }
}

#[test]
fn required_system_time_params_do_not_derive_default() {
    for backend in [Postgres::Sync, Postgres::Tokio, Postgres::Deadpool] {
        let tokens = generated(backend, echo_timestamp_query(), 0);

        assert!(tokens.contains("pub struct EchoTimestampParams"));
        assert!(tokens.contains("std :: time :: SystemTime"));
        assert!(!tokens.contains("Default"));
    }
}

#[test]
fn defaultable_timestamp_override_params_derive_default() {
    for backend in [Postgres::Sync, Postgres::Tokio, Postgres::Deadpool] {
        let mut type_map = backend.db_type_map();
        type_map.insert_db_type(
            "timestamp",
            query::RsType::new(syn::parse_str("crate::Timestamp").unwrap(), None, true)
                .with_can_default(true),
        );
        let tokens = generated_tokens_with_type_map(backend, &type_map, echo_timestamp_query(), 0)
            .to_string();

        assert!(tokens.contains("# [derive (Debug , Clone , Default)]"));
    }
}

#[test]
fn non_defaultable_timestamp_override_params_do_not_derive_default() {
    for backend in [Postgres::Sync, Postgres::Tokio, Postgres::Deadpool] {
        let mut type_map = backend.db_type_map();
        type_map.insert_db_type(
            "timestamp",
            query::RsType::new(syn::parse_str("crate::Timestamp").unwrap(), None, true)
                .with_can_default(false),
        );
        let tokens = generated_tokens_with_type_map(backend, &type_map, echo_timestamp_query(), 0)
            .to_string();

        assert!(!tokens.contains("# [derive (Debug , Clone , Default)]"));
    }
}

#[test]
fn dynamic_execresult_uses_the_count_execution_arm() {
    for backend in [Postgres::Sync, Postgres::Tokio, Postgres::Deadpool] {
        let tokens = generated(
            backend,
            query(
                "TouchAuthors",
                ":execresult",
                "UPDATE authors SET id = id WHERE id = $1 -- :if @id",
                Vec::new(),
                vec![(1, column("id", false))],
            ),
            1,
        );
        assert!(tokens.contains("touch_authors"));
        assert!(tokens.contains("Result < u64"));
        assert!(!tokens.contains("prepare_touch_authors"));
    }
}

#[test]
fn uses_deadpool_paths_and_cached_statements() {
    let deadpool = generated(Postgres::Deadpool, static_query(), 1);
    assert!(deadpool.contains("prepare_cached"));
    assert!(deadpool.contains("deadpool_postgres :: GenericClient"));
    assert!(deadpool.contains("deadpool_postgres :: tokio_postgres :: types :: ToSql"));

    let tokio = generated(Postgres::Tokio, static_query(), 1);
    assert!(!tokio.contains("prepare_cached"));
}

#[test]
fn deadpool_to_sql_path() {
    assert_eq!(
        Postgres::Deadpool.paths().to_sql.to_string(),
        "deadpool_postgres :: tokio_postgres :: types :: ToSql"
    );
}

#[test]
fn embedded_rows_decode_by_select_ordinal() {
    let type_map = Postgres::Tokio.db_type_map();
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

    for backend in [Postgres::Sync, Postgres::Tokio, Postgres::Deadpool] {
        let tokens = params_common::ParamsGenerator::returning_row(&backend, &row).to_string();
        assert!(tokens.contains("before : row . try_get (0) ?"));
        assert!(tokens.contains("id : row . try_get (1) ?"));
        assert!(tokens.contains("name : row . try_get (2) ?"));
    }
}

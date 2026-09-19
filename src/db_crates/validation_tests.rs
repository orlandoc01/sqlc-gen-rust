use crate::{
    db_crates::{
        DbCrate, Postgres, Sqlx,
        params_common::ParamsGenerator,
        rusqlite::Rusqlite,
        test_support::{column, dialect, identifier, parsed, query},
    },
    plugin,
    query::{Annotation, Query},
};

fn generate(
    backend: DbCrate,
    plugin_queries: Vec<plugin::Query>,
    catalog: Option<&plugin::Catalog>,
) -> Result<proc_macro2::TokenStream, crate::query::QueryError> {
    let (rows, queries) = parsed(backend, &backend.db_type_map(), catalog, &plugin_queries);
    backend.generate_queries(
        &rows,
        &queries,
        &crate::db_crates::GenerateOptions::default(),
    )
}

fn generated_functions(
    backend: DbCrate,
    query: &Query,
) -> Vec<super::params_common::GeneratedItem> {
    match backend {
        DbCrate::Sqlx(sqlx) => sqlx.generated_functions(query),
        DbCrate::Rusqlite => Rusqlite.generated_functions(query),
        DbCrate::Postgres(postgres) => postgres.generated_functions(query),
    }
}

fn parsed_query(backend: DbCrate, annotation: Annotation, dynamic: bool) -> Query {
    let sql = if dynamic {
        match dialect(backend) {
            crate::dynfilter::Dialect::PostgreSql => {
                "SELECT id FROM authors WHERE id = $1 -- :if @id"
            }
            crate::dynfilter::Dialect::MySql => "SELECT id FROM authors WHERE id = ? -- :if @id",
            crate::dynfilter::Dialect::Sqlite => "SELECT id FROM authors WHERE id = ?1 -- :if @id",
        }
    } else {
        "SELECT id FROM authors WHERE id = $1"
    };
    let plugin_query = query(
        "Example",
        &annotation.to_string(),
        sql,
        vec![column("id", false)],
        vec![(1, column("id", false))],
    );
    parsed(backend, &backend.db_type_map(), None, &[plugin_query])
        .1
        .remove(0)
}

fn assert_collision(
    backend: DbCrate,
    queries: Vec<plugin::Query>,
    first: &str,
    second: &str,
    ident: &str,
) {
    let error = generate(backend, queries, None).unwrap_err().to_string();
    assert!(error.contains(&format!("`{first}`")), "{error}");
    assert!(error.contains(&format!("`{second}`")), "{error}");
    assert!(error.contains(&format!("`{ident}`")), "{error}");
}

#[test]
fn rejects_optional_helper_collisions_on_every_backend() {
    for backend in DbCrate::ALL {
        assert_collision(
            backend,
            vec![
                query(
                    "GetAuthorOpt",
                    ":exec",
                    "DELETE FROM authors",
                    Vec::new(),
                    Vec::new(),
                ),
                query(
                    "GetAuthor",
                    ":one",
                    "SELECT id FROM authors",
                    vec![column("id", false)],
                    Vec::new(),
                ),
            ],
            "GetAuthor",
            "GetAuthorOpt",
            "get_author_opt",
        );
    }
}

#[test]
fn rejects_normalized_query_name_collisions_on_every_backend() {
    for backend in DbCrate::ALL {
        assert_collision(
            backend,
            vec![
                query(
                    "GetAuthor",
                    ":exec",
                    "DELETE FROM authors",
                    Vec::new(),
                    Vec::new(),
                ),
                query(
                    "get_author",
                    ":exec",
                    "DELETE FROM authors",
                    Vec::new(),
                    Vec::new(),
                ),
            ],
            "GetAuthor",
            "get_author",
            "get_author",
        );
    }
}

#[test]
fn reserves_prepare_helpers_only_for_postgres_drivers() {
    let queries = || {
        vec![
            query(
                "GetAuthor",
                ":one",
                "SELECT id FROM authors",
                vec![column("id", false)],
                Vec::new(),
            ),
            query(
                "PrepareGetAuthor",
                ":exec",
                "DELETE FROM authors",
                Vec::new(),
                Vec::new(),
            ),
        ]
    };

    for backend in [
        DbCrate::Sqlx(Sqlx::Postgres),
        DbCrate::Sqlx(Sqlx::MySql),
        DbCrate::Sqlx(Sqlx::Sqlite),
        DbCrate::Rusqlite,
    ] {
        assert!(generate(backend, queries(), None).is_ok());
    }
    for backend in [
        DbCrate::Postgres(Postgres::Sync),
        DbCrate::Postgres(Postgres::Tokio),
        DbCrate::Postgres(Postgres::Deadpool),
    ] {
        assert_collision(
            backend,
            queries(),
            "GetAuthor",
            "PrepareGetAuthor",
            "prepare_get_author",
        );
    }
}

#[test]
fn rejects_generated_constant_and_dynamic_plan_collisions_on_every_backend() {
    let static_query = query(
        "SearchEntriesDyn",
        ":many",
        "SELECT id FROM entries",
        vec![column("id", false)],
        Vec::new(),
    );

    for backend in DbCrate::ALL {
        let dynamic = query(
            "SearchEntries",
            ":many",
            match dialect(backend) {
                crate::dynfilter::Dialect::PostgreSql => {
                    "SELECT id FROM entries WHERE id = $1 -- :if @id"
                }
                crate::dynfilter::Dialect::MySql => {
                    "SELECT id FROM entries WHERE id = ? -- :if @id"
                }
                crate::dynfilter::Dialect::Sqlite => {
                    "SELECT id FROM entries WHERE id = ?1 -- :if @id"
                }
            },
            vec![column("id", false)],
            vec![(1, column("id", false))],
        );
        for queries in [
            vec![dynamic.clone(), static_query.clone()],
            vec![static_query.clone(), dynamic.clone()],
        ] {
            let error = generate(backend, queries, None).unwrap_err().to_string();
            assert!(error.contains("`SearchEntries`"), "{error}");
            assert!(error.contains("`SearchEntriesDyn`"), "{error}");
            assert!(error.contains("`SEARCH_ENTRIES_DYN`"), "{error}");
            assert!(error.contains("dynamic plan"), "{error}");
            assert!(error.contains("SQL constant"), "{error}");
        }

        let error = generate(
            backend,
            vec![query(
                "Queries",
                ":exec",
                "DELETE FROM entries",
                Vec::new(),
                Vec::new(),
            )],
            None,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("query index"), "{error}");
        assert!(error.contains("SQL constant"), "{error}");
        assert!(error.contains("`QUERIES`"), "{error}");
    }
}

#[test]
fn generated_functions_match_backend_annotation_support() {
    let annotations = [
        Annotation::Exec,
        Annotation::ExecResult,
        Annotation::ExecRows,
        Annotation::ExecLastId,
        Annotation::Many,
        Annotation::One,
        Annotation::BatchExec,
        Annotation::BatchMany,
        Annotation::BatchOne,
        Annotation::CopyFrom,
    ];

    for backend in DbCrate::ALL {
        for annotation in annotations {
            for dynamic in [false, true] {
                let query = parsed_query(backend, annotation, dynamic);
                assert_eq!(
                    !generated_functions(backend, &query).is_empty(),
                    backend.supports(annotation),
                    "{backend:?} {annotation} dynamic={dynamic}"
                );
            }
        }
    }
}

fn matrix_column() -> plugin::Column {
    let mut matrix = column("matrix", false);
    matrix.array_dims = 2;
    matrix
}

fn matrix_catalog() -> plugin::Catalog {
    plugin::Catalog {
        comment: String::new(),
        default_schema: String::new(),
        name: String::new(),
        schemas: vec![plugin::Schema {
            comment: String::new(),
            name: String::new(),
            tables: vec![plugin::Table {
                rel: Some(identifier("matrices")),
                columns: vec![matrix_column()],
                comment: String::new(),
            }],
            enums: Vec::new(),
            composite_types: Vec::new(),
        }],
    }
}

fn assert_array_dimensions_rejected(
    backend: DbCrate,
    plugin_query: plugin::Query,
    catalog: Option<&plugin::Catalog>,
    query_name: &str,
) {
    assert_eq!(
        generate(backend, vec![plugin_query], catalog)
            .unwrap_err()
            .to_string(),
        format!(
            "PostgreSQL backend supports one-dimensional arrays only: query `{query_name}`, column `matrix`"
        )
    );
}

#[test]
fn postgres_drivers_reject_multidimensional_array_parameters_and_rows() {
    let catalog = matrix_catalog();
    for backend in [
        DbCrate::Postgres(Postgres::Sync),
        DbCrate::Postgres(Postgres::Tokio),
        DbCrate::Postgres(Postgres::Deadpool),
    ] {
        assert_array_dimensions_rejected(
            backend,
            query(
                "InsertMatrix",
                ":exec",
                "INSERT INTO matrices (matrix) VALUES ($1)",
                Vec::new(),
                vec![(1, matrix_column())],
            ),
            None,
            "InsertMatrix",
        );
        assert_array_dimensions_rejected(
            backend,
            query(
                "GetMatrix",
                ":one",
                "SELECT matrix FROM matrices",
                vec![matrix_column()],
                Vec::new(),
            ),
            None,
            "GetMatrix",
        );
        let mut embedded = column("matrices", false);
        embedded.embed_table = Some(identifier("matrices"));
        assert_array_dimensions_rejected(
            backend,
            query(
                "GetEmbeddedMatrix",
                ":one",
                "SELECT sqlc.embed(matrices) FROM matrices",
                vec![embedded],
                Vec::new(),
            ),
            Some(&catalog),
            "GetEmbeddedMatrix",
        );
    }
}

/// The fixture resolves gated-join references against the supplied catalog the way
/// `generate` does: `email` belongs only to the ungated table, so it needs no gate.
#[test]
fn fixture_resolves_gated_joins_against_the_catalog() {
    let table = |name: &str, columns: &[&str]| plugin::Table {
        rel: Some(identifier(name)),
        columns: columns.iter().map(|name| column(name, false)).collect(),
        comment: String::new(),
    };
    let catalog = plugin::Catalog {
        schemas: vec![plugin::Schema {
            tables: vec![
                table("users", &["id", "email"]),
                table("orders", &["id", "user_id", "created_at"]),
            ],
            ..Default::default()
        }],
        ..Default::default()
    };
    let plugin_query = query(
        "ListUsers",
        ":many",
        "SELECT u.id FROM users u\nLEFT JOIN orders o ON o.user_id = u.id -- :flag @with_orders\nWHERE email <> ''\nORDER BY u.id",
        vec![column("id", false)],
        Vec::new(),
    );
    generate(
        DbCrate::Sqlx(Sqlx::Postgres),
        vec![plugin_query],
        Some(&catalog),
    )
    .unwrap();
}

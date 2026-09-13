use crate::{
    db_crates::{
        DbCrate, Postgres, Sqlx,
        test_support::{column, identifier, query},
    },
    plugin,
    query::{Query, ReturnRowAttributes, ReturningRows},
};

fn generate(
    backend: DbCrate,
    plugin_queries: Vec<plugin::Query>,
    catalog: Option<&plugin::Catalog>,
) -> Result<proc_macro2::TokenStream, crate::query::QueryError> {
    let type_map = backend.db_type_map();
    let (rows, queries): (Vec<_>, Vec<_>) = plugin_queries
        .into_iter()
        .map(|plugin_query| {
            let row = ReturningRows::from_query(
                &type_map,
                &ReturnRowAttributes::default(),
                catalog,
                &plugin_query,
            )
            .unwrap();
            let query = Query::from_query(&type_map, &plugin_query).unwrap();
            (row, query)
        })
        .unzip();
    backend.generate_queries(&rows, &queries, 1)
}

fn backends() -> [DbCrate; 7] {
    [
        DbCrate::Sqlx(Sqlx::Postgres),
        DbCrate::Sqlx(Sqlx::MySql),
        DbCrate::Sqlx(Sqlx::Sqlite),
        DbCrate::Rusqlite,
        DbCrate::Postgres(Postgres::Sync),
        DbCrate::Postgres(Postgres::Tokio),
        DbCrate::Postgres(Postgres::Deadpool),
    ]
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
    for backend in backends() {
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
    for backend in backends() {
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

use crate::{
    db_crates::{
        DbCrate, Postgres, params_common,
        sqlx::Sqlx,
        test_support::{column, parsed, query},
    },
    plugin,
};

fn collision_error(backend: Postgres, first: plugin::Query, second: plugin::Query) -> String {
    let type_map = backend.db_type_map();
    let (rows, queries) = parsed(
        DbCrate::Postgres(backend),
        &type_map,
        None,
        &[first, second],
    );
    params_common::generate_queries(
        &backend,
        &rows,
        &queries,
        &crate::db_crates::GenerateOptions::default(),
    )
    .unwrap_err()
    .to_string()
}

#[test]
fn rejects_prepare_function_name_collisions() {
    let error = collision_error(
        Postgres::Tokio,
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
        "Queries `GetAuthor` (prepare helper) and `PrepareGetAuthor` (query function) both generate Rust item `prepare_get_author`"
    );
}

#[test]
fn rejects_with_function_name_collisions() {
    let error = collision_error(
        Postgres::Tokio,
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
        "Queries `GetAuthor` (with helper) and `GetAuthorWith` (query function) both generate Rust item `get_author_with`"
    );
}

#[test]
fn rejects_stream_function_name_collisions() {
    let error = collision_error(
        Postgres::Tokio,
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
        "Queries `ListAuthors` (stream helper) and `ListAuthorsStream` (query function) both generate Rust item `list_authors_stream`"
    );
}

#[test]
fn rejects_dynamic_helper_name_collisions() {
    let error = collision_error(
        Postgres::Tokio,
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
        "Queries `SearchAuthors` (dynamic query helper) and `SearchAuthorsQuery` (query function) both generate Rust item `search_authors_query`"
    );
}

#[test]
fn rejects_iter_function_name_collisions_for_sync_postgres() {
    let error = collision_error(
        Postgres::Sync,
        query(
            "ListAuthors",
            ":many",
            "SELECT id FROM authors",
            vec![column("id", false)],
            vec![],
        ),
        query(
            "ListAuthorsIter",
            ":exec",
            "DELETE FROM authors",
            Vec::new(),
            vec![],
        ),
    );

    assert_eq!(
        error,
        "Queries `ListAuthors` (iter helper) and `ListAuthorsIter` (query function) both generate Rust item `list_authors_iter`"
    );
}

#[test]
fn rejects_iter_with_function_name_collisions_for_sync_postgres() {
    let error = collision_error(
        Postgres::Sync,
        query(
            "ListAuthors",
            ":many",
            "SELECT id FROM authors",
            vec![column("id", false)],
            vec![],
        ),
        query(
            "ListAuthorsIterWith",
            ":exec",
            "DELETE FROM authors",
            Vec::new(),
            vec![],
        ),
    );

    assert_eq!(
        error,
        "Queries `ListAuthors` (iter with helper) and `ListAuthorsIterWith` (query function) both generate Rust item `list_authors_iter_with`"
    );
}

#[test]
fn rejects_dynamic_iter_function_name_collisions_for_sync_postgres() {
    let error = collision_error(
        Postgres::Sync,
        query(
            "SearchAuthors",
            ":many",
            "SELECT id FROM authors WHERE id = $1 -- :if @id",
            vec![column("id", false)],
            vec![(1, column("id", false))],
        ),
        query(
            "SearchAuthorsIter",
            ":exec",
            "DELETE FROM authors",
            Vec::new(),
            vec![],
        ),
    );

    assert_eq!(
        error,
        "Queries `SearchAuthors` (iter helper) and `SearchAuthorsIter` (query function) both generate Rust item `search_authors_iter`"
    );
}

#[test]
fn reserves_only_the_backend_iterator_suffix() {
    let sync = Postgres::Sync;
    let sync_type_map = sync.db_type_map();
    let (rows, queries) = parsed(
        DbCrate::Postgres(sync),
        &sync_type_map,
        None,
        &[
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
        ],
    );
    assert!(
        params_common::generate_queries(
            &sync,
            &rows,
            &queries,
            &crate::db_crates::GenerateOptions::default()
        )
        .is_ok()
    );

    let tokio = Postgres::Tokio;
    let tokio_type_map = tokio.db_type_map();
    let (rows, queries) = parsed(
        DbCrate::Postgres(tokio),
        &tokio_type_map,
        None,
        &[
            query(
                "ListAuthors",
                ":many",
                "SELECT id FROM authors",
                vec![column("id", false)],
                vec![],
            ),
            query(
                "ListAuthorsIter",
                ":exec",
                "DELETE FROM authors",
                Vec::new(),
                vec![],
            ),
        ],
    );
    assert!(
        params_common::generate_queries(
            &tokio,
            &rows,
            &queries,
            &crate::db_crates::GenerateOptions::default()
        )
        .is_ok()
    );
}

#[test]
fn does_not_reserve_tokio_helpers_for_sqlx() {
    let type_map = Sqlx::Sqlite.db_type_map();
    let (rows, queries) = parsed(
        DbCrate::Sqlx(Sqlx::Sqlite),
        &type_map,
        None,
        &[
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

    assert!(
        params_common::generate_queries(
            &Sqlx::Sqlite,
            &rows,
            &queries,
            &crate::db_crates::GenerateOptions::default()
        )
        .is_ok()
    );
}

use crate::{
    db_crates::{
        DbCrate, Postgres,
        sqlx::Sqlx,
        test_support::{column, generate, query},
    },
    plugin,
};

fn collision_error(backend: Postgres, first: plugin::Query, second: plugin::Query) -> String {
    generate(
        DbCrate::Postgres(backend),
        &backend.db_type_map(),
        None,
        &[first, second],
        1,
        None,
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

fn generates_ok(backend: DbCrate, queries: &[plugin::Query]) -> bool {
    generate(backend, &backend.db_type_map(), None, queries, 1, None).is_ok()
}

#[test]
fn reserves_only_the_backend_iterator_suffix() {
    assert!(generates_ok(
        DbCrate::Postgres(Postgres::Sync),
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
    ));
    assert!(generates_ok(
        DbCrate::Postgres(Postgres::Tokio),
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
    ));
}

#[test]
fn does_not_reserve_tokio_helpers_for_sqlx() {
    assert!(generates_ok(
        DbCrate::Sqlx(Sqlx::Sqlite),
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
    ));
}

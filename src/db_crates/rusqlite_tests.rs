use crate::{
    db_crates::{
        DbCrate,
        rusqlite::Rusqlite,
        test_support::{column, generate, query},
    },
    plugin,
};

fn generated(query: plugin::Query, query_parameter_limit: usize) -> String {
    generate(
        DbCrate::Rusqlite,
        &Rusqlite.db_type_map(),
        None,
        &[query],
        query_parameter_limit,
        None,
    )
    .unwrap()
    .to_string()
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
            "SELECT id FROM authors WHERE id = ?1 -- :if @id",
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

fn switch_query(name: &str, sql: &str) -> plugin::Query {
    query(
        name,
        ":many",
        sql,
        vec![column("id", false)],
        vec![(1, column("owner_id", false))],
    )
}

const SWITCHED_SQL: &str = "SELECT id FROM notes\nWHERE\n  owner_id = ?1 -- :if @owner_id\n  AND archived = 0 -- :flag @hide_archived\nORDER BY -- :switch @sort newest oldest title default=newest\n  created_at DESC, -- :case @newest\n  created_at ASC, -- :case @oldest\n  title ASC, -- :case @title\n  id ASC -- :case @title\nLIMIT 10";

#[test]
fn generates_a_switch_enum_and_matches_lowering() {
    let tokens = generated(switch_query("ListNotes", SWITCHED_SQL), 1);

    assert!(
        tokens.contains(
            "# [derive (Debug , Clone , Copy , PartialEq , Eq , Default)] pub enum ListNotesSort { # [default] Newest , Oldest , Title , }"
        ),
        "{tokens}"
    );
    assert!(
        tokens.contains(
            "pub struct ListNotesParams { pub owner_id : Option < i64 > , pub hide_archived : bool , pub sort : ListNotesSort , }"
        ),
        "{tokens}"
    );
    assert!(!tokens.contains("pub newest : bool"));
    assert!(
        tokens.contains(
            "dynfilter :: Arg :: from_option (& params . owner_id) , dynfilter :: Arg :: Flag (params . hide_archived) , dynfilter :: Arg :: Flag (matches ! (params . sort , ListNotesSort :: Newest)) , dynfilter :: Arg :: Flag (matches ! (params . sort , ListNotesSort :: Oldest)) , dynfilter :: Arg :: Flag (matches ! (params . sort , ListNotesSort :: Title)) ,"
        ),
        "{tokens}"
    );
}

fn generation_error(queries: Vec<plugin::Query>) -> String {
    generate(
        DbCrate::Rusqlite,
        &Rusqlite.db_type_map(),
        None,
        &queries,
        1,
        None,
    )
    .unwrap_err()
    .to_string()
}

#[test]
fn rejects_generated_identifier_collisions() {
    let error = generation_error(vec![switch_query(
        "ListNotes",
        "SELECT id FROM notes\nORDER BY -- :switch @sort id_asc IdAsc default=id_asc\n  id ASC, -- :case @id_asc\n  id DESC -- :case @IdAsc\nLIMIT 10",
    )]);
    assert!(
        error.contains("`id_asc` and `IdAsc` both generate switch variant `IdAsc`"),
        "{error}"
    );

    let error = generation_error(vec![switch_query(
        "ListNotes",
        "SELECT id FROM notes\nWHERE\n  owner_id = ?1 -- :if @owner_id\n  AND a = 1 -- :flag @SortBy\n  AND b = 1 -- :flag @sort_by\nLIMIT 10",
    )]);
    assert!(
        error.contains("`SortBy` and `sort_by` both generate params struct field `sort_by`"),
        "{error}"
    );

    let error = generation_error(vec![
        switch_query(
            "Search",
            "SELECT id FROM notes\nORDER BY -- :switch @users_params a b default=a\n  id ASC, -- :case @a\n  id DESC -- :case @b\nLIMIT 10",
        ),
        switch_query(
            "SearchUsers",
            "SELECT id FROM notes WHERE owner_id = ?1 -- :if @owner_id\n",
        ),
    ]);
    assert!(
        error.contains("both generate Rust item `SearchUsersParams`"),
        "{error}"
    );

    let error = generation_error(vec![switch_query(
        "SearchUsers",
        "SELECT id FROM notes\nORDER BY -- :switch @row a b default=a\n  id ASC, -- :case @a\n  id DESC -- :case @b\nLIMIT 10",
    )]);
    assert!(
        error.contains(
            "(row struct) and `SearchUsers` (switch enum) both generate Rust item `SearchUsersRow`"
        ),
        "{error}"
    );
}

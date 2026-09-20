use super::*;
use crate::db_crates::test_support::{column, identifier};

#[test]
fn raw_string_literals_decode_to_the_original_text() {
    for text in [
        "SELECT 1",
        "SELECT '\"' AS quote",
        "SELECT '\"#' AS quote_hash",
        "SELECT '\"##\"#' AS quote_hashes",
        "SELECT '#\"' AS hash_quote",
        "SELECT '\\' AS backslash",
        "SELECT 1\nFROM t\n  WHERE a = $1",
        "SELECT '\u{e9}' AS accent",
        "SELECT 1\r\nFROM t",
    ] {
        let literal = syn::parse2::<syn::LitStr>(super::raw_string_literal(text))
            .unwrap_or_else(|error| panic!("{text:?}: {error}"));
        assert_eq!(literal.value(), text, "{text:?}");
    }
    assert!(
        super::raw_string_literal("SELECT '\"#'\nFROM t")
            .to_string()
            .starts_with("r##\"")
    );
    assert_eq!(
        super::raw_string_literal("SELECT 1\nFROM t").to_string(),
        "r\"SELECT 1\nFROM t\""
    );
}

fn create_test_column(table_name: Option<&str>, column_name: &str) -> plugin::Column {
    plugin::Column {
        name: column_name.to_string(),
        table: table_name.map(|name| plugin::Identifier {
            name: name.to_string(),
            schema: String::new(),
            catalog: String::new(),
        }),
        not_null: false,
        is_array: false,
        comment: String::new(),
        length: 0,
        is_named_param: false,
        is_func_call: false,
        scope: String::new(),
        table_alias: String::new(),
        r#type: None,
        is_sqlc_slice: false,
        embed_table: None,
        original_name: String::new(),
        unsigned: false,
        array_dims: 0,
    }
}

fn test_catalog() -> plugin::Catalog {
    plugin::Catalog {
        comment: String::new(),
        default_schema: String::new(),
        name: String::new(),
        schemas: vec![plugin::Schema {
            comment: String::new(),
            name: String::new(),
            tables: vec![
                plugin::Table {
                    rel: Some(identifier("authors")),
                    columns: vec![column("id", false), column("name", false)],
                    comment: String::new(),
                },
                plugin::Table {
                    rel: Some(identifier("books")),
                    columns: vec![
                        column("id", false),
                        column("author_id", false),
                        column("title", false),
                    ],
                    comment: String::new(),
                },
            ],
            enums: Vec::new(),
            composite_types: Vec::new(),
        }],
    }
}

fn test_type_map() -> DbTypeMap {
    let mut type_map = SimpleTypeMap::default();
    type_map.insert_db_type(
        "integer",
        RsType::new(syn::parse_str("i64").unwrap(), None, true),
    );
    DbTypeMap::from_dyn(Box::new(type_map))
}

#[test]
fn test_empty_columns() {
    let columns = vec![create_test_column(None, ""), create_test_column(None, "")];

    let names = generate_column_names(&columns);
    assert_eq!(names, vec!["column_1", "column_2"]);
}

#[test]
fn test_unique_column_names() {
    let columns = vec![
        create_test_column(None, "id"),
        create_test_column(None, "name"),
    ];

    let names = generate_column_names(&columns);
    assert_eq!(names, vec!["id", "name"]);
}

#[test]
fn test_duplicate_column_names_different_tables() {
    let columns = vec![
        create_test_column(Some("users"), "id"),
        create_test_column(Some("posts"), "id"),
    ];

    let names = generate_column_names(&columns);
    assert_eq!(names, vec!["users_id", "posts_id"]);
}

#[test]
fn test_duplicate_table_and_column() {
    let columns = vec![
        create_test_column(Some("users"), "id"),
        create_test_column(Some("users"), "id"),
    ];

    let names = generate_column_names(&columns);
    assert_eq!(names, vec!["users_id_1", "users_id_2"]);
}

#[test]
fn test_mixed_scenarios() {
    let columns = vec![
        create_test_column(None, ""),            // column_1
        create_test_column(None, "name"),        // name (unique)
        create_test_column(Some("users"), "id"), // users_id_1 (重複するので連番)
        create_test_column(Some("posts"), "id"), // posts_id (重複しないのでそのまま)
        create_test_column(None, "id"),          // id (重複しないのでそのまま)
        create_test_column(Some("users"), "id"), // users_id_2 (重複するので連番)
    ];

    let names = generate_column_names(&columns);
    assert_eq!(
        names,
        vec![
            "column_1",
            "name",
            "users_id_1",
            "posts_id",
            "id",
            "users_id_2"
        ]
    );
}

#[test]
fn test_complex_scenario_with_multiple_duplicates() {
    let columns = vec![
        create_test_column(Some("users"), "name"), // users_name_1
        create_test_column(Some("posts"), "name"), // posts_name
        create_test_column(Some("users"), "name"), // users_name_2
        create_test_column(None, "name"),          // name_1
        create_test_column(None, "name"),          // name_2
    ];

    let names = generate_column_names(&columns);
    assert_eq!(
        names,
        vec![
            "users_name_1",
            "posts_name",
            "users_name_2",
            "name_1",
            "name_2"
        ]
    );
}

#[test]
fn finds_embedded_table_in_catalog() {
    let catalog = test_catalog();

    let table = find_embedded_table(&catalog, &identifier("authors")).unwrap();
    assert_eq!(table.rel.as_ref().unwrap().name, "authors");

    let error = find_embedded_table(&catalog, &identifier("missing")).unwrap_err();
    assert_eq!(
        error.to_string(),
        "Embedded table not found in catalog: `missing`"
    );
}

#[test]
fn assigns_embed_ordinals_in_flattened_order() {
    let catalog = test_catalog();
    let mut authors = create_test_column(None, "authors");
    authors.embed_table = Some(identifier("authors"));
    let mut books = create_test_column(None, "books");
    books.embed_table = Some(identifier("books"));
    let query = plugin::Query {
        text: String::new(),
        name: "Embedded".to_string(),
        cmd: ":one".to_string(),
        columns: vec![
            column("before", false),
            authors,
            column("after", false),
            books,
        ],
        params: Vec::new(),
        comments: Vec::new(),
        filename: String::new(),
        insert_into_table: None,
    };

    let row = ReturningRows::from_query(
        &test_type_map(),
        &ReturnRowAttributes::default(),
        Some(&catalog),
        &query,
    )
    .unwrap();

    assert_eq!(
        row.field_ordinals().collect::<Vec<_>>(),
        vec![0..1, 1..3, 3..4, 4..7]
    );
}

#[test]
fn dynamic_filter_errors_name_the_query() {
    let plugin_query = plugin::Query {
        text: "SELECT id -- :if @id\nFROM authors WHERE id = $1".to_string(),
        name: "BadFilter".to_string(),
        cmd: ":many".to_string(),
        columns: vec![column("id", false)],
        params: vec![plugin::Parameter {
            number: 1,
            column: Some(column("id", false)),
        }],
        comments: Vec::new(),
        filename: String::new(),
        insert_into_table: None,
    };
    let Err(error) = Query::parse(
        &test_type_map(),
        &plugin_query,
        crate::dynfilter::Dialect::PostgreSql,
        &Default::default(),
        false,
    ) else {
        panic!("BadFilter parsed");
    };

    assert!(error.to_string().contains("Query `BadFilter`"));
    assert!(error.to_string().contains("does not attach"));
}

#[test]
fn rejects_gapped_or_duplicate_parameter_numbers() {
    let backend = crate::db_crates::DbCrate::default();
    for numbers in [vec![1, 3], vec![2], vec![1, 1]] {
        let params = numbers
            .iter()
            .map(|number| (*number, column("id", false)))
            .collect();
        let plugin = crate::db_crates::test_support::query(
            "Gapped",
            ":many",
            "SELECT id FROM t WHERE id = $1 AND b = $3",
            vec![column("id", false)],
            params,
        );
        let error = match Query::from_query(&backend.db_type_map(), &plugin) {
            Ok(_) => panic!("{numbers:?} accepted"),
            Err(error) => error.to_string(),
        };
        assert!(error.contains("not 1..="), "{numbers:?}: {error}");
    }
}

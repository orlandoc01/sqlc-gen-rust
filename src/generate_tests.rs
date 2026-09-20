use super::*;

fn dynamic_filter_request() -> plugin::GenerateRequest {
    let column = plugin::Column {
        name: "id".to_string(),
        not_null: true,
        r#type: Some(plugin::Identifier {
            name: "integer".to_string(),
            ..Default::default()
        }),
        ..Default::default()
    };
    plugin::GenerateRequest {
        settings: Some(plugin::Settings {
            engine: "postgresql".to_string(),
            ..Default::default()
        }),
        queries: vec![plugin::Query {
            text: "SELECT id -- :if @id\nFROM users WHERE id = $1".to_string(),
            name: "BadFilter".to_string(),
            cmd: ":many".to_string(),
            columns: vec![column.clone()],
            params: vec![plugin::Parameter {
                number: 1,
                column: Some(column),
            }],
            ..Default::default()
        }],
        plugin_options: br#"{"db_crate":"sqlx-postgres"}"#.to_vec(),
        ..Default::default()
    }
}

#[test]
fn parses_db_crates_and_rejects_legacy_api() {
    let config = Config::from_option(br#"{"api":"builder"}"#).unwrap();
    assert_eq!(
        config.validate(&[]).unwrap_err().to_string(),
        "the builder API was removed; only params_struct is generated"
    );

    assert_eq!(
        Config::from_option(br#"{"api":"params_struct"}"#)
            .unwrap()
            .api
            .as_deref(),
        Some("params_struct")
    );
}

#[test]
fn structural_rejection_is_reported_by_generation() {
    let error = generate(&dynamic_filter_request(), Vec::new()).unwrap_err();
    assert!(error.to_string().contains("Query `BadFilter`"));
}

fn switch_request(sql: &str) -> plugin::GenerateRequest {
    let mut request = dynamic_filter_request();
    let query = &mut request.queries[0];
    query.name = "SearchUsers".to_string();
    query.text = sql.to_string();
    request
}

#[test]
fn control_names_that_cannot_become_rust_identifiers_are_rejected() {
    for (sql, expected) in [
        (
            "SELECT id FROM users\nORDER BY -- :switch @sort _1st other default=other\n  id ASC, -- :case @_1st\n  id DESC -- :case @other\nLIMIT 1",
            "cannot be turned into a Rust identifier",
        ),
        (
            "SELECT id FROM users\nWHERE id = $1 -- :flag @_\nLIMIT 1",
            "cannot be turned into a Rust identifier",
        ),
        (
            "SELECT id FROM users\nORDER BY -- :switch @sort self other default=other\n  id ASC, -- :case @self\n  id DESC -- :case @other\nLIMIT 1",
            "reserved Rust identifier `Self`",
        ),
    ] {
        let error = generate(&switch_request(sql), Vec::new()).unwrap_err();
        assert!(error.to_string().contains(expected), "{sql}: {error}");
    }

    let response = generate(
        &switch_request("SELECT id FROM users\nWHERE id = 1 -- :flag @type\nLIMIT 1"),
        Vec::new(),
    )
    .unwrap();
    let code = String::from_utf8(response.files[0].contents.clone()).unwrap();
    assert!(code.contains("pub r#type: bool"), "{code}");
}

#[test]
fn switch_enums_cannot_shadow_catalog_enums_or_backend_items() {
    let mut request = switch_request(
        "SELECT id FROM users\nORDER BY -- :switch @sort id_asc id_desc default=id_asc\n  id ASC, -- :case @id_asc\n  id DESC -- :case @id_desc\nLIMIT 1",
    );
    request.queries[0].name = "Search".to_string();
    request.catalog = Some(plugin::Catalog {
        schemas: vec![plugin::Schema {
            enums: vec![plugin::Enum {
                name: "search_sort".to_string(),
                vals: vec!["a".to_string(), "b".to_string()],
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    });
    let error = generate(&request, Vec::new()).unwrap_err().to_string();
    assert!(error.contains("defines `SearchSort` twice"), "{error}");

    let mut request = switch_request(
        "SELECT id FROM users\nORDER BY -- :switch @client a b default=a\n  id ASC, -- :case @a\n  id DESC -- :case @b\nLIMIT 1",
    );
    request.queries[0].name = "Rusqlite".to_string();
    request.settings.as_mut().unwrap().engine = "sqlite".to_string();
    request.plugin_options = br#"{"db_crate":"rusqlite"}"#.to_vec();
    let error = generate(&request, Vec::new()).unwrap_err().to_string();
    assert!(error.contains("defines `RusqliteClient` twice"), "{error}");

    let mut request = switch_request(
        "SELECT id FROM users\nORDER BY -- :switch @sort id_asc id_desc default=id_asc\n  id ASC, -- :case @id_asc\n  id DESC -- :case @id_desc\nLIMIT 1",
    );
    request.catalog = Some(plugin::Catalog {
        schemas: vec![plugin::Schema {
            enums: vec![plugin::Enum {
                name: "user_kind".to_string(),
                vals: vec!["a".to_string()],
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    });
    assert!(generate(&request, Vec::new()).is_ok());
}

#[test]
fn unknown_annotation_keywords_are_rejected_by_generation() {
    let error = generate(
        &switch_request("SELECT id FROM users\nORDER BY id ASC -- :sort @id_asc\nLIMIT 1"),
        Vec::new(),
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Query `SearchUsers`: annotation on line 2: unknown annotation `-- :sort`"),
        "{error}"
    );
}

#[test]
fn switches_generate_an_enum_field() {
    let response = generate(
        &switch_request(
            "SELECT id FROM users\nWHERE id = $1 -- :if @id\nORDER BY -- :switch @sort id_asc id_desc default=id_desc\n  id ASC, -- :case @id_asc\n  id DESC -- :case @id_desc\nLIMIT 1",
        ),
        Vec::new(),
    )
    .unwrap();
    let code = String::from_utf8(response.files[0].contents.clone()).unwrap();

    assert!(
        code.contains("pub enum SearchUsersSort {\n    IdAsc,\n    #[default]\n    IdDesc,\n}"),
        "{code}"
    );
    assert!(code.contains("pub sort: SearchUsersSort,"), "{code}");
    assert!(
        code.contains("matches!(params.sort, SearchUsersSort::IdAsc)"),
        "{code}"
    );
}

fn variants_request() -> plugin::GenerateRequest {
    let mut request = switch_request(
        "SELECT id FROM users\nWHERE\n  id = $1 -- :if @id\nORDER BY -- :switch @sort asc desc default=asc\n  id ASC, -- :case @asc\n  id DESC -- :case @desc",
    );
    request.plugin_options =
        br#"{"db_crate":"sqlx-postgres","dynfilters":{"prepared":true}}"#.to_vec();
    request
}

/// sqlc 1.31.1's SQLite engine copies a preceding query's own-line `;` onto the next text.
#[test]
fn strips_the_preceding_query_semicolon_sqlite_copies_into_the_next_text() {
    let mut request = variants_request();
    request.settings.as_mut().unwrap().engine = "sqlite".to_string();
    request.plugin_options = br#"{"db_crate":"sqlx-sqlite"}"#.to_vec();
    let dynamic = &mut request.queries[0];
    dynamic.text = ";

SELECT id FROM users
WHERE
  id = ?1 -- :if @id"
        .to_string();

    let response = generate(&request, Vec::new()).unwrap();
    let code = std::str::from_utf8(&response.files[0].contents).unwrap();
    assert!(
        code.contains("pub const SEARCH_USERS: &str = r\"SELECT id FROM users"),
        "{code}"
    );
}

#[test]
fn plumbs_limit_and_skip_options_through_generate() {
    let mut request = variants_request();
    request.plugin_options =
        br#"{"db_crate":"sqlx-postgres","dynfilters":{"prepared":true,"variant_limit":3}}"#
            .to_vec();
    let error = generate(&request, Vec::new()).unwrap_err().to_string();
    assert_eq!(
        error,
        "Query `SearchUsers`: dynamic-filter variants exceed dynfilters.variant_limit 3; raise dynfilters.variant_limit or add the query to dynfilters.variants_skip"
    );

    request.plugin_options =
        br#"{"db_crate":"sqlx-postgres","dynfilters":{"prepared":true,"variant_limit":3,"variants_skip":["SearchUsers"]}}"#.to_vec();
    let response = generate(&request, Vec::new()).unwrap();
    assert_eq!(response.files.len(), 1);
    let code = std::str::from_utf8(&response.files[0].contents).unwrap();
    assert!(!code.contains("SEARCH_USERS_VARIANTS"), "{code}");
    assert!(
        code.contains("pub const DYNFILTER_VARIANT_COUNT: usize = 0;"),
        "{code}"
    );

    request.plugin_options =
        br#"{"db_crate":"sqlx-postgres","dynfilters":{"prepared":true,"variants_skip":["Nope"]}}"#
            .to_vec();
    assert_eq!(
        generate(&request, Vec::new()).unwrap_err().to_string(),
        "Query `Nope`: dynfilters.variants_skip names no dynamic-filter query"
    );

    // Without the flag nothing is enumerated, so the limit cannot trip.
    request.plugin_options =
        br#"{"db_crate":"sqlx-postgres","dynfilters":{"variant_limit":3}}"#.to_vec();
    let response = generate(&request, Vec::new()).unwrap();
    assert_eq!(response.files.len(), 1);
    let code = std::str::from_utf8(&response.files[0].contents).unwrap();
    assert!(!code.contains("_VARIANTS"), "{code}");
}

#[test]
fn rejects_unsupported_dynamic_filter_engines() {
    let mut request = dynamic_filter_request();
    request.settings.as_mut().unwrap().engine = "oracle".to_string();

    assert!(
        generate(&request, Vec::new())
            .unwrap_err()
            .to_string()
            .contains("do not support SQL engine `oracle`")
    );
}

#[test]
fn parses_rusqlite_db_crate() {
    assert!(matches!(
        Config::from_option(br#"{"db_crate":"rusqlite"}"#)
            .unwrap()
            .db_crate,
        db_crates::DbCrate::Rusqlite
    ));
    assert!(matches!(
        Config::from_option(br#"{"db_crate":"tokio-postgres"}"#)
            .unwrap()
            .db_crate,
        db_crates::DbCrate::Postgres(db_crates::Postgres::Tokio)
    ));
    assert!(matches!(
        Config::from_option(br#"{"db_crate":"deadpool-postgres"}"#)
            .unwrap()
            .db_crate,
        db_crates::DbCrate::Postgres(db_crates::Postgres::Deadpool)
    ));
    assert!(matches!(
        Config::from_option(br#"{"db_crate":"postgres"}"#)
            .unwrap()
            .db_crate,
        db_crates::DbCrate::Postgres(db_crates::Postgres::Sync)
    ));
}

#[test]
fn applies_override_types_and_validates_targets() {
    let config = Config::from_option(
        br#"{"overrides":[{"db_type":"timestamp","rs_type":"crate::Timestamp","copy_cheap":true,"can_default":true}]}"#,
    )
    .unwrap();
    let mut db_type = config.db_crate.db_type_map();
    apply_overrides(&config, &mut db_type).unwrap();
    assert!(db_type.find_rs_type("timestamp").unwrap().can_default());

    for (options, message) in [
        (
            br#"{"overrides":[{"db_type":"timestamp","column":".events.timestamp","rs_type":"crate::Timestamp"}]}"#
                .as_slice(),
            "Cannot override both db_type and column name at the same time.",
        ),
        (
            br#"{"overrides":[{"rs_type":"crate::Timestamp"}]}"#.as_slice(),
            "Must override either db_type or column name.",
        ),
    ] {
        let config = Config::from_option(options).unwrap();
        let mut db_type = config.db_crate.db_type_map();
        assert_eq!(apply_overrides(&config, &mut db_type).unwrap_err().to_string(), message);
    }
}

#[test]
fn rejects_unsupported_params_struct_annotations() {
    let mut query = Query::from_query(
        &db_crates::DbCrate::Rusqlite.db_type_map(),
        &plugin::Query {
            text: "DELETE FROM authors".to_string(),
            name: "DeleteAuthors".to_string(),
            cmd: ":execresult".to_string(),
            columns: Vec::new(),
            params: Vec::new(),
            comments: Vec::new(),
            filename: String::new(),
            insert_into_table: None,
        },
    )
    .unwrap();
    let config = Config::from_option(br#"{"db_crate":"rusqlite"}"#).unwrap();
    assert_eq!(
        config
            .validate(std::slice::from_ref(&query))
            .unwrap_err()
            .to_string(),
        "params_struct does not support :execresult with rusqlite (DeleteAuthors)."
    );

    query.annotation = query::Annotation::CopyFrom;
    assert_eq!(
        config
            .validate(std::slice::from_ref(&query))
            .unwrap_err()
            .to_string(),
        "params_struct does not support :copyfrom with rusqlite (DeleteAuthors)."
    );

    query.annotation = query::Annotation::ExecLastId;
    let config = Config::from_option(br#"{"db_crate":"tokio-postgres"}"#).unwrap();
    assert_eq!(
        config
            .validate(std::slice::from_ref(&query))
            .unwrap_err()
            .to_string(),
        "params_struct does not support :execlastid with tokio-postgres (DeleteAuthors)."
    );

    let config = Config::from_option(br#"{"db_crate":"deadpool-postgres"}"#).unwrap();
    assert_eq!(
        config
            .validate(std::slice::from_ref(&query))
            .unwrap_err()
            .to_string(),
        "params_struct does not support :execlastid with deadpool-postgres (DeleteAuthors)."
    );

    let config = Config::from_option(br#"{"db_crate":"postgres"}"#).unwrap();
    assert_eq!(
        config
            .validate(std::slice::from_ref(&query))
            .unwrap_err()
            .to_string(),
        "params_struct does not support :execlastid with postgres (DeleteAuthors)."
    );
}

fn prepared_request(engine: &str, db_crate: &str, sql: &str) -> plugin::GenerateRequest {
    let mut request = switch_request(sql);
    request.settings.as_mut().unwrap().engine = engine.to_string();
    request.plugin_options =
        format!(r#"{{"db_crate":"{db_crate}","dynfilters":{{"prepared":true}}}}"#).into_bytes();
    request
}

fn const_strings(code: &str, ident: &str) -> Vec<String> {
    let file = syn::parse_file(code).unwrap();
    let item = file
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Const(item) if item.ident == ident => Some(item),
            _ => None,
        })
        .unwrap_or_else(|| panic!("{ident} in {code}"));
    let syn::Expr::Reference(reference) = &*item.expr else {
        panic!("{ident} is not a slice");
    };
    let syn::Expr::Array(array) = &*reference.expr else {
        panic!("{ident} is not an array");
    };
    array
        .elems
        .iter()
        .map(|element| match element {
            syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(text),
                ..
            }) => text.value(),
            _ => panic!("{ident} holds a non-string"),
        })
        .collect()
}

#[test]
fn prepared_options_are_validated() {
    let mut request = switch_request("SELECT id FROM users WHERE id = $1 -- :if @id");
    for option in ["variants_skip", "prepared_skip"] {
        request.plugin_options = format!(
            r#"{{"db_crate":"sqlx-postgres","dynfilters":{{"{option}":["SearchUsers"]}}}}"#
        )
        .into_bytes();
        assert_eq!(
            generate(&request, Vec::new()).unwrap_err().to_string(),
            format!("dynfilters.{option} requires dynfilters.prepared: true")
        );
    }
    request.plugin_options =
        br#"{"db_crate":"sqlx-postgres","dynfilters":{"prepared":true,"prepared_skip":["Nope"]}}"#
            .to_vec();
    assert_eq!(
        generate(&request, Vec::new()).unwrap_err().to_string(),
        "Query `Nope`: dynfilters.prepared_skip names no dynamic-filter query"
    );
    for options in [&b"{}"[..], br#"{"dynfilters":{}}"#] {
        let dynfilters = Config::from_option(options).unwrap().dynfilters;
        assert!(!dynfilters.prepared);
        assert!(dynfilters.prepared_skip.is_empty());
        assert!(dynfilters.variants_skip.is_empty());
        assert_eq!(dynfilters.variant_limit, 1024);
    }
}

#[test]
fn prepared_artifacts_appear_in_one_pass() {
    let request = prepared_request(
        "postgresql",
        "sqlx-postgres",
        "SELECT id FROM users\nWHERE\n  id = $1 -- :if @id\nORDER BY -- :switch @sort asc desc default=asc\n  id ASC, -- :case @asc\n  id DESC -- :case @desc",
    );
    let response = generate(&request, Vec::new()).unwrap();
    assert_eq!(response.files.len(), 1);
    let code = std::str::from_utf8(&response.files[0].contents).unwrap();
    assert!(!code.contains("persistent"), "{code}");
    assert!(
        code.contains("pub const DYNFILTER_VARIANT_COUNT: usize = 4;"),
        "{code}"
    );
    assert!(
        code.contains("self::prepare_search_users(conn).await?;"),
        "{code}"
    );
    assert_eq!(const_strings(code, "SEARCH_USERS_VARIANTS").len(), 4);
}

#[test]
fn sqlite_prepared_texts_match_the_runtime_byte_for_byte() {
    let request = prepared_request(
        "sqlite",
        "sqlx-sqlite",
        "SELECT id, '?2 $9 -- :if @id' AS label\nFROM users\nWHERE\n  id = ?1 -- :if @id\nORDER BY -- :switch @sort asc desc default=asc\n  id ASC, -- :case @asc\n  id DESC -- :case @desc",
    );
    let response = generate(&request, Vec::new()).unwrap();
    let code = std::str::from_utf8(&response.files[0].contents).unwrap();
    let variants = const_strings(code, "SEARCH_USERS_VARIANTS");

    let type_map = db_crates::DbCrate::Sqlx(db_crates::Sqlx::Sqlite).db_type_map();
    let query = Query::parse(
        &type_map,
        &request.queries[0],
        dynfilter::Dialect::Sqlite,
        &Default::default(),
        true,
    )
    .unwrap();
    let info = query.expect_dynfilter();
    let compiled = dynfilter_runtime::compile_with_arg_order(
        &info.annotated_sql,
        dynfilter_runtime::Placeholders::NumberedSqlite,
        dynfilter_runtime::Dialect::Sqlite,
        query.arg_order(),
        &info.runtime_plan(),
    );
    use dynfilter_runtime::{Arg, Bind};
    let mut built = Vec::new();
    for id in [Arg::Inactive, Arg::Active] {
        for asc in [true, false] {
            let (sql, binds) = compiled.build(&[id, Arg::Flag(asc), Arg::Flag(!asc)]);
            let expected_binds = matches!(id, Arg::Active)
                .then(|| vec![Bind::Arg(0)])
                .unwrap_or_default();
            assert_eq!(binds, expected_binds, "{sql}");
            built.push(sql);
        }
    }
    assert_eq!(variants, built);
    assert!(
        variants[3].contains(
            "'?2 $9 -- :if @id' AS label\nFROM users\nWHERE\n  id = $1\nORDER BY\n  id DESC"
        ),
        "{:?}",
        variants[3]
    );
    assert!(code.contains("for sql in SEARCH_USERS_VARIANTS {\n        sqlx::Executor::prepare(&mut *conn, sql).await?;\n    }"), "{code}");
}

#[test]
fn packages_without_eligible_queries_keep_an_empty_aggregate() {
    let mut request = prepared_request(
        "sqlite",
        "rusqlite",
        "SELECT id FROM users WHERE id IN (/*SLICE:ids*/?1)",
    );
    request.queries[0].params[0]
        .column
        .as_mut()
        .unwrap()
        .is_sqlc_slice = true;
    let response = generate(&request, Vec::new()).unwrap();
    assert_eq!(response.files.len(), 1);
    let code = std::str::from_utf8(&response.files[0].contents).unwrap();
    assert!(code.contains("pub fn prepare_dynfilter_variants(\n    _client: &impl RusqliteClient,\n) -> rusqlite::Result<()> {\n    Ok(())\n}"), "{code}");
    assert!(
        code.contains("pub const DYNFILTER_VARIANT_COUNT: usize = 0;"),
        "{code}"
    );
    assert!(code.contains(".prepare(&sql)?"), "{code}");
}

#[test]
fn prepared_skip_names_must_have_controls_and_valid_skips_disable_caching_only() {
    let mut request = prepared_request(
        "sqlite",
        "rusqlite",
        "SELECT id FROM users\nWHERE\n  id = ?1 -- :if @id",
    );
    request.queries.push(plugin::Query {
        name: "Fixed".to_string(),
        text: "SELECT id FROM users WHERE id = ?1".to_string(),
        ..request.queries[0].clone()
    });
    let mut slice = plugin::Query {
        name: "ByIds".to_string(),
        text: "SELECT id FROM users WHERE id IN (/*SLICE:ids*/?1)".to_string(),
        ..request.queries[0].clone()
    };
    slice.params[0].column.as_mut().unwrap().is_sqlc_slice = true;
    request.queries.push(slice);
    for name in ["Fixed", "ByIds"] {
        request.plugin_options = format!(
            r#"{{"db_crate":"rusqlite","dynfilters":{{"prepared":true,"prepared_skip":["{name}"]}}}}"#
        )
        .into_bytes();
        assert_eq!(
            generate(&request, Vec::new()).unwrap_err().to_string(),
            format!("Query `{name}`: dynfilters.prepared_skip names no dynamic-filter query")
        );
    }

    request.plugin_options =
        br#"{"db_crate":"rusqlite","dynfilters":{"prepared":true,"prepared_skip":["SearchUsers"]}}"#.to_vec();
    let response = generate(&request, Vec::new()).unwrap();
    let code = std::str::from_utf8(&response.files[0].contents).unwrap();
    assert!(!code.contains("SEARCH_USERS_VARIANTS"), "{code}");
    assert!(
        code.contains("pub const DYNFILTER_VARIANT_COUNT: usize = 0;"),
        "{code}"
    );
    assert_eq!(code.matches(".prepare(&sql)?").count(), 2, "{code}");
}

/// Aliases that differ only by punctuation or case normalize to one Rust member; `syn` would
/// accept the duplicate, so generation must refuse it wherever members are emitted.
#[test]
fn rejects_members_whose_rust_spellings_collide() {
    let column_named = |name: &str| plugin::Column {
        name: name.to_string(),
        ..dynamic_filter_request().queries[0].columns[0].clone()
    };
    let mut request = switch_request("SELECT id AS \"a-b\", id AS a_b FROM users WHERE id = $1");
    request.queries[0].columns = vec![column_named("a-b"), column_named("a_b")];
    assert_eq!(
        generate(&request, Vec::new()).unwrap_err().to_string(),
        "Query `SearchUsers`: `a-b` and `a_b` both generate row field `a_b`"
    );
    // A control: distinct spellings still generate.
    request.queries[0].columns = vec![column_named("a-b"), column_named("b_a")];
    let code = generate(&request, Vec::new()).unwrap();
    assert!(
        std::str::from_utf8(&code.files[0].contents)
            .unwrap()
            .contains("pub b_a:")
    );

    let param = |number: i32, name: &str| plugin::Parameter {
        number,
        column: Some(column_named(name)),
    };
    for (label, sql, options) in [
        (
            "static parameters",
            "SELECT id FROM users WHERE id = $1 AND id = $2",
            br#"{"db_crate":"sqlx-postgres","query_parameter_limit":0}"#.to_vec(),
        ),
        (
            "direct parameters",
            "SELECT id FROM users WHERE id = $1 AND id = $2",
            br#"{"db_crate":"sqlx-postgres","query_parameter_limit":5}"#.to_vec(),
        ),
        (
            "dynamic parameters",
            "SELECT id FROM users WHERE id = $1 -- :if @a_b\n  AND id = $2 -- :if @a-b",
            br#"{"db_crate":"sqlx-postgres"}"#.to_vec(),
        ),
    ] {
        let mut request = switch_request(sql);
        request.queries[0].columns = vec![column_named("id")];
        request.queries[0].params = vec![param(1, "a_b"), param(2, "a-b")];
        request.plugin_options = options;
        assert_eq!(
            generate(&request, Vec::new()).unwrap_err().to_string(),
            "Query `SearchUsers`: `a_b` and `a-b` both generate parameter `a_b`",
            "{label}"
        );
    }

    let mut request = switch_request("SELECT sqlc.embed(users) FROM users WHERE id = $1");
    request.queries[0].columns = vec![plugin::Column {
        embed_table: Some(plugin::Identifier {
            name: "users".to_string(),
            ..Default::default()
        }),
        ..column_named("users")
    }];
    request.catalog = Some(plugin::Catalog {
        schemas: vec![plugin::Schema {
            tables: vec![plugin::Table {
                rel: Some(plugin::Identifier {
                    name: "users".to_string(),
                    ..Default::default()
                }),
                columns: vec![column_named("a-b"), column_named("a_b")],
                comment: String::new(),
            }],
            enums: vec![plugin::Enum {
                name: "status".to_string(),
                vals: vec!["on-hold".to_string(), "on_hold".to_string()],
                comment: String::new(),
            }],
            ..Default::default()
        }],
        ..Default::default()
    });
    assert_eq!(
        generate(&request, Vec::new()).unwrap_err().to_string(),
        "Enum `status`: `on-hold` and `on_hold` both generate variant `OnHold`"
    );
    request.catalog.as_mut().unwrap().schemas[0].enums.clear();
    assert_eq!(
        generate(&request, Vec::new()).unwrap_err().to_string(),
        "Embedded table `users`: `a-b` and `a_b` both generate field `a_b`"
    );
}

/// A second positional parameter, `email`, after the request's `id`.
fn with_email_param(mut request: plugin::GenerateRequest) -> plugin::GenerateRequest {
    let query = &mut request.queries[0];
    let mut column = query.params[0].column.clone().unwrap();
    column.name = "email".to_string();
    query.params.push(plugin::Parameter {
        number: 2,
        column: Some(column),
    });
    request
}

/// PostgreSQL brackets are expressions: every parameter inside binds, and a conditional bind
/// after them takes the next number.
#[test]
fn postgres_array_constructors_keep_their_binds() {
    let request = with_email_param(prepared_request(
        "postgresql",
        "sqlx-postgres",
        "SELECT id FROM users\nWHERE\n  id = ANY(ARRAY[$1::bigint, $1::bigint])\n  AND email = $2 -- :if @email",
    ));
    let response = generate(&request, Vec::new()).unwrap();
    let code = std::str::from_utf8(&response.files[0].contents).unwrap();
    assert_eq!(
        const_strings(code, "SEARCH_USERS_VARIANTS"),
        [
            "SELECT id FROM users\nWHERE\n  id = ANY(ARRAY[$1::bigint, $1::bigint])",
            "SELECT id FROM users\nWHERE\n  id = ANY(ARRAY[$1::bigint, $1::bigint])\n  AND email = $2",
        ]
    );
}

/// A `$` inside an identifier is not a parameter reference, so the SQLx PostgreSQL bind-type
/// classifier never sees a phantom argument.
#[test]
fn dollar_identifiers_survive_prepared_generation() {
    let request = prepared_request(
        "postgresql",
        "sqlx-postgres",
        "SELECT price$9, x$1 FROM users\nWHERE\n  id = $1 -- :if @id",
    );
    let response = generate(&request, Vec::new()).unwrap();
    let code = std::str::from_utf8(&response.files[0].contents).unwrap();
    assert_eq!(
        const_strings(code, "SEARCH_USERS_VARIANTS"),
        [
            "SELECT price$9, x$1 FROM users",
            "SELECT price$9, x$1 FROM users\nWHERE\n  id = $1",
        ]
    );
}

#[test]
fn comment_prose_outside_dynamic_clauses_survives_in_variants() {
    let request = prepared_request(
        "postgresql",
        "sqlx-postgres",
        "-- app:users\nSELECT id -- Documentation example: -- :if $1\n\nFROM users\nWHERE id = $1 -- :if @id",
    );
    let response = generate(&request, Vec::new()).unwrap();
    let code = std::str::from_utf8(&response.files[0].contents).unwrap();
    assert_eq!(
        const_strings(code, "SEARCH_USERS_VARIANTS"),
        [
            "-- app:users\nSELECT id -- Documentation example: -- :if $1\n\nFROM users",
            "-- app:users\nSELECT id -- Documentation example: -- :if $1\n\nFROM users\nWHERE\n  id = $1",
        ]
    );
}

#[test]
fn unknown_dynfilters_keys_are_rejected() {
    let error =
        Config::from_option(br#"{"dynfilters":{"prepared":true,"prepared_skp":["SearchUsers"]}}"#)
            .unwrap_err()
            .to_string();
    assert!(error.contains("prepared_skp"), "{error}");
    let config = Config::from_option(
        br#"{"dynfilters":{"prepared":true,"prepared_skip":["A"],"variant_limit":7,"variants_skip":["B"]}}"#,
    )
    .unwrap();
    assert!(config.dynfilters.prepared);
    assert_eq!(config.dynfilters.prepared_skip, ["A"]);
    assert_eq!(config.dynfilters.variant_limit, 7);
    assert_eq!(config.dynfilters.variants_skip, ["B"]);

    let mut request = switch_request("SELECT id FROM users\nWHERE id = $1 -- :if @id");
    request.plugin_options =
        br#"{"db_crate":"sqlx-postgres","dynfilters":{"prepared":true,"prepared_skp":["SearchUsers"]}}"#
            .to_vec();
    let error = generate(&request, Vec::new()).unwrap_err().to_string();
    assert!(error.contains("prepared_skp"), "{error}");
    request.plugin_options =
        br#"{"db_crate":"sqlx-postgres","dynfilters":{"prepared":true,"prepared_skip":["SearchUsers"]}}"#
            .to_vec();
    let response = generate(&request, Vec::new()).unwrap();
    let code = std::str::from_utf8(&response.files[0].contents).unwrap();
    assert!(!code.contains("SEARCH_USERS_VARIANTS"), "{code}");
    assert!(code.contains("persistent(false)"), "{code}");
}

/// `users(id, email)` and `orders(id, user_id, created_at)`.
fn users_orders_catalog() -> plugin::Catalog {
    let table = |name: &str, columns: &[&str]| plugin::Table {
        rel: Some(plugin::Identifier {
            name: name.to_string(),
            ..Default::default()
        }),
        columns: columns
            .iter()
            .map(|name| plugin::Column {
                name: name.to_string(),
                ..dynamic_filter_request().queries[0].columns[0].clone()
            })
            .collect(),
        comment: String::new(),
    };
    plugin::Catalog {
        schemas: vec![plugin::Schema {
            tables: vec![
                table("users", &["id", "email"]),
                table("orders", &["id", "user_id", "created_at"]),
            ],
            ..Default::default()
        }],
        ..Default::default()
    }
}

/// The catalog decides whether an unqualified column beside a gated join is safe, in every
/// mode, including for queries excluded from enumeration or caching.
#[test]
fn gated_join_references_resolve_against_the_catalog() {
    let sql = |predicate: &str| {
        format!(
            "SELECT u.id FROM users u\nLEFT JOIN orders o ON o.user_id = u.id -- :flag @with_orders\nWHERE {predicate} AND u.id = $1\nORDER BY u.id"
        )
    };
    for options in [
        r#"{"db_crate":"sqlx-postgres"}"#,
        r#"{"db_crate":"sqlx-postgres","dynfilters":{"prepared":true}}"#,
        r#"{"db_crate":"sqlx-postgres","dynfilters":{"prepared":true,"variants_skip":["SearchUsers"]}}"#,
        r#"{"db_crate":"sqlx-postgres","dynfilters":{"prepared":true,"prepared_skip":["SearchUsers"]}}"#,
    ] {
        let mut request = switch_request(&sql("email <> ''"));
        request.catalog = Some(users_orders_catalog());
        request.plugin_options = options.as_bytes().to_vec();
        generate(&request, Vec::new()).unwrap_or_else(|error| panic!("{options}: {error}"));

        request.queries[0].text = sql("created_at <> ''");
        let error = generate(&request, Vec::new()).unwrap_err().to_string();
        assert!(
            error.contains("created_at") && error.contains("with_orders"),
            "{options}: {error}"
        );
    }
}

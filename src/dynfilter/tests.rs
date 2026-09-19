//! Tests for directive grammar, control slot allocation, and static slice numbering.

use super::*;

#[test]
fn numbers_static_slice_markers() {
    let info = parse_static_slices(
        "SELECT * FROM users WHERE id IN (/*SLICE:ids*/?7)",
        Dialect::Sqlite,
        &[("ids".to_string(), 2)],
    )
    .unwrap()
    .unwrap();

    assert_eq!(
        info.annotated_sql,
        "SELECT * FROM users WHERE id IN (/*SLICE:ids*/?2)"
    );
    assert!(info.flag_params.is_empty());
    assert!(
        parse_static_slices("SELECT 1", Dialect::Sqlite, &[])
            .unwrap()
            .is_none()
    );
}

#[test]
fn numbers_only_slice_marker_tokens() {
    let sql = "SELECT '/*SLICE:ids*/?' AS label, \"/*SLICE:ids*/?\", `/*SLICE:ids*/?` /* /*SLICE:ids*/? */ FROM users -- /*SLICE:ids*/?\nWHERE id IN (/*SLICE:ids*/?) AND kind IN (/*SLICE:kinds*/?) AND /*SLICE:ids*/ ? = 1";
    let info = parse_static_slices(
        sql,
        Dialect::MySql,
        &[("ids".to_string(), 1), ("kinds".to_string(), 2)],
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        info.annotated_sql,
        "SELECT '/*SLICE:ids*/?' AS label, \"/*SLICE:ids*/?\", `/*SLICE:ids*/?` /* /*SLICE:ids*/? */ FROM users -- /*SLICE:ids*/?\nWHERE id IN (/*SLICE:ids*/?1) AND kind IN (/*SLICE:kinds*/?2) AND /*SLICE:ids*/ ? = 1"
    );
    let sqlite = parse_static_slices(
        "SELECT '/*SLICE:ids*/?' AS label FROM users WHERE id IN (/*SLICE:ids*/?)",
        Dialect::Sqlite,
        &[("ids".to_string(), 1)],
    )
    .unwrap()
    .unwrap();
    assert!(sqlite.annotated_sql.contains("'/*SLICE:ids*/?' AS label"));
    assert!(sqlite.annotated_sql.ends_with("(/*SLICE:ids*/?1)"));
}

fn switch(field: &str, choices: &[&str], default: &str) -> Switch {
    Switch {
        field: field.into(),
        choices: choices.iter().map(|choice| choice.to_string()).collect(),
        default: default.into(),
    }
}

#[test]
fn parses_every_annotation_keyword() {
    assert_eq!(
        parse_directive(" :if @a $b").unwrap(),
        Directive::If(vec!["a".into(), "b".into()])
    );
    assert_eq!(
        parse_directive(":flag @on").unwrap(),
        Directive::Flag(vec!["on".into()])
    );
    assert_eq!(
        parse_directive(" :case @mine").unwrap(),
        Directive::Case("mine".into())
    );
    assert_eq!(
        parse_directive(" :switch @sort id_asc default=id_desc id_desc").unwrap(),
        Directive::Switch(switch("sort", &["id_asc", "id_desc"], "id_desc"))
    );
}

#[test]
fn rejects_identifiers_starting_with_a_digit() {
    for text in [
        " :switch @sort 1st 2nd default=1st",
        " :switch @1sort a b default=a",
        " :flag @1st",
        " :if @1st",
        " :case @1st",
    ] {
        assert!(parse_directive(text).is_err(), "{text}");
    }
    assert!(parse_directive(" :flag @_1st").is_ok());
}

#[test]
fn rejects_unknown_and_malformed_annotations() {
    for (text, expected) in [
        (" :", "`-- :` is missing a keyword"),
        (" :!", "`-- :` is missing a keyword"),
        (" : if @x", "`-- :` is missing a keyword"),
        (" :sort @x", "unknown annotation `-- :sort`"),
        (" :if", "`-- :if` expects one or more `@name`"),
        (" :if email", "`-- :if` expects one or more `@name`"),
        (" :flag @a b", "`-- :flag` expects one or more `@name`"),
        (" :case @a @b", "`-- :case` takes exactly one `@choice`"),
        (" :switch", "malformed switch"),
        (
            " :switch sort a b default=a",
            "switch field must be written `@field`",
        ),
        (" :switch @sort a default=a", "needs at least two choices"),
        (" :switch @sort a a default=a", "declares choice `a` twice"),
        (" :switch @sort a b", "needs exactly one `default=choice`"),
        (
            " :switch @sort a b default=a default=b",
            "needs exactly one `default=choice`",
        ),
        (
            " :switch @sort a b default=c",
            "default `c` is not one of its choices",
        ),
        (
            " :switch @sort a @b default=a",
            "choice `@b` is not a bare identifier",
        ),
    ] {
        let error = parse_directive(text).unwrap_err();
        assert!(error.contains(expected), "{text}: {error}");
    }
}

#[test]
fn allocates_controls_after_every_sql_argument_even_with_duplicate_names() {
    let params = [("id".to_string(), 1), ("id".to_string(), 2)];
    let directives = [Directive::Flag(vec!["hide_archived".into()])];
    let resolved = references(&params, directives.iter()).unwrap();

    assert_eq!(resolved.arg_index_by_name["hide_archived"], 2);

    let error = references(&params, [Directive::If(vec!["id".into()])].iter()).unwrap_err();
    assert!(error.contains("ambiguous"), "{error}");
}

#[test]
fn classifies_directives_into_conditions_flags_and_switch_slots() {
    let params = [("email".to_string(), 1), ("limit".to_string(), 2)];
    let directives = [
        Directive::If(vec!["email".into()]),
        Directive::Flag(vec!["has_orders".into()]),
        Directive::Switch(switch("sort", &["id_asc", "id_desc"], "id_asc")),
        Directive::Flag(vec!["has_orders".into(), "active".into()]),
        Directive::Case("id_desc".into()),
    ];
    let references = references(&params, directives.iter()).unwrap();

    assert_eq!(references.conditional_param_numbers, [1]);
    assert_eq!(
        references
            .flag_params
            .iter()
            .map(|flag| (flag.name.as_str(), flag.switch))
            .collect::<Vec<_>>(),
        [
            ("has_orders", None),
            ("id_asc", Some(0)),
            ("id_desc", Some(0)),
            ("active", None)
        ]
    );
    assert_eq!(
        references.switches,
        [switch("sort", &["id_asc", "id_desc"], "id_asc")]
    );
    assert_eq!(references.arg_index_by_name["email"], 0);
    assert_eq!(references.arg_index_by_name["has_orders"], 2);
    assert_eq!(references.arg_index_by_name["id_desc"], 4);
    assert_eq!(references.arg_index_by_name["active"], 5);
    assert!(!references.arg_index_by_name.contains_key("sort"));
}

#[test]
fn rejects_misclassified_names() {
    let params = [("email".to_string(), 1)];
    for (directives, expected) in [
        (
            vec![Directive::If(vec!["has_orders".into()])],
            "`-- :if @has_orders` names no SQL parameter; use `-- :flag @has_orders`",
        ),
        (
            vec![Directive::Flag(vec!["email".into()])],
            "`email` is a SQL parameter and cannot be a `-- :flag` name; use `-- :if @email`",
        ),
        (
            vec![Directive::Switch(switch("email", &["a", "b"], "a"))],
            "`email` is a SQL parameter and cannot be a `-- :switch` field",
        ),
        (
            vec![Directive::Switch(switch("sort", &["email", "b"], "b"))],
            "`email` is a SQL parameter and cannot be a `-- :switch` choice",
        ),
        (
            vec![
                Directive::Flag(vec!["a".into()]),
                Directive::Switch(switch("sort", &["a", "b"], "a")),
            ],
            "`a` is already a `-- :flag` name; it cannot also be a `-- :switch` choice",
        ),
        (
            vec![
                Directive::Switch(switch("sort", &["a", "b"], "a")),
                Directive::Flag(vec!["sort".into()]),
            ],
            "`sort` is already a `-- :switch` field; it cannot also be a `-- :flag` name",
        ),
        (
            vec![
                Directive::Switch(switch("sort", &["a", "b"], "a")),
                Directive::Switch(switch("scope", &["a", "c"], "c")),
            ],
            "`a` is already a `-- :switch` choice; it cannot also be a `-- :switch` choice",
        ),
    ] {
        let error = references(&params, directives.iter()).unwrap_err();
        assert!(error.contains(expected), "{error}");
    }
}

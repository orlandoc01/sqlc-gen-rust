//! Parser tests for `-- :if` and `-- :flag` on WHERE conjuncts and ORDER BY terms.

mod filters {
    use crate::dynfilter::test_support::{compile, params, parse_uncatalogued};
    use crate::dynfilter::{Connector, Dialect, FlagParam, PlanClause};
    use crate::dynfilter_runtime::{Arg, Bind, Placeholders};
    use sqlparser::parser::Parser;
    use sqlparser::tokenizer::{Token, Tokenizer, Whitespace};

    #[test]
    fn malformed_directives_are_diagnosed_not_panics() {
        for (sql, line) in [
            ("SELECT id FROM users\nWHERE id > 0 -- :flag @x\n-- :", 3),
            ("SELECT id FROM users\nWHERE id > 0 -- :flag @x\n-- :!", 3),
            ("SELECT id FROM users\nWHERE id > 0\n-- :", 3),
            ("SELECT id FROM users -- : prose\nWHERE id > 0", 1),
            ("SELECT id FROM users\nWHERE id > 0 -- :if", 2),
        ] {
            let error = parse_uncatalogued(sql, &params(&[]), Dialect::PostgreSql).unwrap_err();
            assert!(
                error.starts_with(&format!("annotation on line {line}:")),
                "{sql}\n{error}"
            );
        }
        assert!(
            parse_uncatalogued(
                "SELECT ': -- :' FROM users -- see the :if docs\nWHERE id > 0",
                &params(&[]),
                Dialect::PostgreSql
            )
            .unwrap()
            .is_none()
        );
    }

    #[test]
    fn tokenizes_placeholders_and_slice_markers_for_each_dialect() {
        for (dialect, placeholder) in [
            (Dialect::PostgreSql, "$1"),
            (Dialect::MySql, "?"),
            (Dialect::Sqlite, "?1"),
        ] {
            let sql = format!("SELECT /*SLICE:ids*/{placeholder}");
            let tokens = Tokenizer::new(dialect.sqlparser(), &sql)
                .tokenize_with_location()
                .unwrap();
            assert!(
                tokens
                    .iter()
                    .any(|token| matches!(token.token, Token::Placeholder(_)))
            );
            assert!(tokens.iter().any(|token| {
                matches!(
                    &token.token,
                    Token::Whitespace(Whitespace::MultiLineComment(comment)) if comment == "SLICE:ids"
                )
            }));
        }
    }

    #[test]
    fn canonicalizes_where_order_and_nested_exists() {
        let info = parse_uncatalogued(
            "SELECT id FROM users\nWHERE email = $1 -- :if @email\n  AND EXISTS ( -- :flag @has_orders\n    SELECT 1 FROM orders\n    WHERE orders.user_id = users.id\n      AND orders.created_at >= $2 -- :if @orders_since\n  )\nORDER BY id ASC, -- :flag @id_asc\n  id DESC -- :flag @id_desc\nLIMIT $3",
            &params(&["email", "orders_since", "limit"]),
            Dialect::PostgreSql,
        )
        .unwrap()
        .unwrap();

        assert_eq!(
            info.flag_params,
            [
                FlagParam {
                    name: "has_orders".into(),
                    switch: None
                },
                FlagParam {
                    name: "id_asc".into(),
                    switch: None
                },
                FlagParam {
                    name: "id_desc".into(),
                    switch: None
                }
            ]
        );
        assert!(info.annotated_sql.contains("WHERE\n  email = $1 -- :if $1"));
        assert!(info.annotated_sql.contains("AND EXISTS ( -- :if $4"));
        assert!(
            info.annotated_sql
                .contains("AND orders.created_at >= $2 -- :if $4 -- :if $2")
        );
        assert!(
            info.annotated_sql.contains("ORDER BY\n  id ASC, -- :if $5"),
            "{}",
            info.annotated_sql
        );
        assert_eq!(
            info.plan,
            [
                PlanClause {
                    header: 1,
                    connector: Connector::And,
                    items: vec![2, 3]
                },
                PlanClause {
                    header: 5,
                    connector: Connector::And,
                    items: vec![6, 7]
                },
                PlanClause {
                    header: 9,
                    connector: Connector::Comma,
                    items: vec![10, 11]
                }
            ],
            "{}",
            info.annotated_sql
        );
    }

    #[test]
    fn keeps_all_active_search_users_sql_valid() {
        let info = parse_uncatalogued(
            "SELECT id, email, phone\nFROM users\nWHERE\n  email = $1 -- :if @email\n  AND phone = $2 -- :if @phone\n  AND EXISTS ( -- :flag @has_orders\n    SELECT 1 FROM orders\n    WHERE orders.user_id = users.id\n      AND orders.created_at >= $3 -- :if @orders_since\n  )\n  AND id = ANY($4::bigint[]) -- :if @ids\nORDER BY\n  id ASC, -- :flag @id_asc\n  id DESC -- :flag @id_desc\nLIMIT $5\n;",
            &params(&["email", "phone", "orders_since", "ids", "row_limit"]),
            Dialect::PostgreSql,
        )
        .unwrap()
        .unwrap();
        let compiled = compile(&info, Placeholders::Numbered, &[1, 2, 3, 4, 5]);
        let (sql, _) = compiled.build(&[
            Arg::Active,
            Arg::Active,
            Arg::Active,
            Arg::Active,
            Arg::Active,
            Arg::Flag(true),
            Arg::Flag(true),
            Arg::Flag(true),
        ]);

        assert_eq!(
            sql,
            "SELECT id, email, phone\nFROM users\nWHERE\n  email = $1\n  AND phone = $2\n  AND EXISTS (\n    SELECT 1 FROM orders\n    WHERE\n      orders.user_id = users.id\n      AND orders.created_at >= $3\n  )\n  AND id = ANY($4::bigint[])\nORDER BY\n  id ASC,\n  id DESC\nLIMIT $5\n;"
        );
        Parser::parse_sql(Dialect::PostgreSql.sqlparser(), &sql).unwrap();
    }

    #[test]
    fn ignores_quoted_identifiers_named_like_keywords() {
        let info = parse_uncatalogued(
            "SELECT id FROM users\nWHERE\n  \"where\" = $1 -- :if @a\n  AND \"order\" = $2 -- :if @b\n  AND \"group\" > 0\nORDER BY\n  \"limit\" ASC, -- :flag @c\n  \"for\" DESC",
            &params(&["a", "b"]),
            Dialect::PostgreSql,
        )
        .unwrap()
        .unwrap();
        let compiled = compile(&info, Placeholders::Numbered, &[1, 2]);

        assert_eq!(
            compiled
                .build(&[Arg::Inactive, Arg::Active, Arg::Flag(false)])
                .0,
            "SELECT id FROM users\nWHERE\n  \"order\" = $1\n  AND \"group\" > 0\nORDER BY\n  \"for\" DESC"
        );
    }

    #[test]
    fn renders_multi_line_order_by_terms() {
        let info = parse_uncatalogued(
            "SELECT * FROM t\nORDER BY\n  CASE WHEN status = $1 THEN 0\n       ELSE 1 END ASC, -- :if @status\n  created_at DESC, -- :flag @newest\n  id ASC",
            &params(&["status"]),
            Dialect::PostgreSql,
        )
        .unwrap()
        .unwrap();

        assert_eq!(
            info.annotated_sql,
            "SELECT * FROM t\nORDER BY\n  CASE WHEN status = $1 THEN 0 -- :if $1\n       ELSE 1 END ASC, -- :if $1\n  created_at DESC, -- :if $2\n  id ASC"
        );
        assert_eq!(
            info.plan,
            [PlanClause {
                header: 1,
                connector: Connector::Comma,
                items: vec![2, 4, 5]
            }]
        );
        let compiled = compile(&info, Placeholders::Numbered, &[1]);
        for (args, expected) in [
            (
                [Arg::Active, Arg::Flag(true)],
                "SELECT * FROM t\nORDER BY\n  CASE WHEN status = $1 THEN 0\n       ELSE 1 END ASC,\n  created_at DESC,\n  id ASC",
            ),
            (
                [Arg::Inactive, Arg::Flag(true)],
                "SELECT * FROM t\nORDER BY\n  created_at DESC,\n  id ASC",
            ),
            (
                [Arg::Active, Arg::Flag(false)],
                "SELECT * FROM t\nORDER BY\n  CASE WHEN status = $1 THEN 0\n       ELSE 1 END ASC,\n  id ASC",
            ),
            (
                [Arg::Inactive, Arg::Flag(false)],
                "SELECT * FROM t\nORDER BY\n  id ASC",
            ),
        ] {
            assert_eq!(compiled.build(&args).0, expected);
        }
    }

    #[test]
    fn rejects_non_structural_annotations() {
        for sql in [
            "SELECT id -- :if @id\nFROM users",
            "SELECT id FROM users GROUP BY id -- :if @id",
            "SELECT id FROM users HAVING COUNT(*) > 1 -- :if @id",
            "SELECT id FROM users LIMIT $1 -- :if @id",
            "SELECT id FROM users WHERE email = $1 -- :if @id\n OR phone = $1",
        ] {
            assert!(
                parse_uncatalogued(sql, &params(&["id"]), Dialect::PostgreSql).is_err(),
                "{sql}"
            );
        }
    }

    #[test]
    fn attaches_a_final_slice_conjunct() {
        let info = parse_uncatalogued(
            ";\nSELECT id FROM users\nWHERE\n  email = ?1 -- :if @email\n  AND id IN (/*SLICE:ids*/?2) -- :if @ids",
            &params(&["email", "ids"]),
            Dialect::Sqlite,
        )
        .unwrap()
        .unwrap();
        assert!(
            info.annotated_sql.contains("?2) -- :if $2"),
            "{}",
            info.annotated_sql
        );
    }

    #[test]
    fn attaches_first_middle_last_and_standalone_structures() {
        let info = parse_uncatalogued(
            "SELECT id FROM users\nWHERE\n  -- :if @first\n  (email = $1 OR phone = $1)\n  AND status = $2 -- :if @middle\n  -- :if @last\n  AND active = $3\nORDER BY\n  id ASC, -- :if @first_order\n  -- :if @last_order\n  email DESC\nLIMIT $6",
            &params(&["first", "middle", "last", "first_order", "last_order", "limit"]),
            Dialect::PostgreSql,
        )
        .unwrap()
        .unwrap();

        assert_eq!(info.conditional_param_numbers, [1, 2, 3, 4, 5]);
        assert!(info.annotated_sql.contains("phone = $1) -- :if $1"));
        assert!(info.annotated_sql.contains("status = $2 -- :if $2"));
        assert!(info.annotated_sql.contains("active = $3 -- :if $3"));
        assert!(info.annotated_sql.contains("id ASC, -- :if $4"));
        assert!(info.annotated_sql.contains("email DESC -- :if $5"));
    }

    #[test]
    fn standalone_annotation_selects_the_outer_exists_conjunct() {
        for (dialect, placeholder) in [(Dialect::PostgreSql, "$1"), (Dialect::MySql, "?")] {
            let sql = format!(
                "SELECT id FROM users\nWHERE\n  -- :flag @has_orders\n  EXISTS (SELECT 1 FROM orders WHERE orders.user_id = users.id AND orders.paid = {placeholder})"
            );
            let info = parse_uncatalogued(&sql, &params(&["paid"]), dialect)
                .unwrap()
                .unwrap();

            assert!(
                info.annotated_sql
                    .lines()
                    .find(|line| line.contains("EXISTS"))
                    .is_some_and(|line| line.contains("-- :if $2")),
                "{}",
                info.annotated_sql
            );
            assert_eq!(info.annotated_sql.matches("-- :if").count(), 1);
        }
    }

    #[test]
    fn attaches_an_annotation_after_any_opening_parenthesis() {
        let info = parse_uncatalogued(
            "SELECT id FROM users\nWHERE\n  id IN ( -- :flag @ids\n    SELECT user_id FROM orders\n  )",
            &params(&[]),
            Dialect::PostgreSql,
        )
        .unwrap()
        .unwrap();

        assert!(info.annotated_sql.contains("id IN ( -- :if $1"));
    }

    #[test]
    fn parses_full_search_users_for_postgres_and_mysql() {
        let postgres = parse_uncatalogued(
            "SELECT id, email, phone\nFROM users\nWHERE\n  email = $1 -- :if @email\n  AND phone = $2 -- :if @phone\n  AND EXISTS ( -- :flag @has_orders\n    SELECT 1 FROM orders\n    WHERE orders.user_id = users.id\n      AND orders.created_at >= $3 -- :if @orders_since\n  )\n  AND users.id = ANY($4::bigint[]) -- :if @ids\nORDER BY\n  users.id ASC, -- :flag @id_asc\n  users.id DESC -- :flag @id_desc\nLIMIT $5",
            &params(&["email", "phone", "orders_since", "ids", "row_limit"]),
            Dialect::PostgreSql,
        )
        .unwrap()
        .unwrap();
        let mysql = parse_uncatalogued(
            "SELECT id, email, phone\nFROM users\nWHERE\n  email = ? -- :if @email\n  AND phone = ? -- :if @phone\n  AND EXISTS ( -- :flag @has_orders\n    SELECT 1 FROM orders\n    WHERE orders.user_id = users.id\n      AND orders.created_at >= ? -- :if @orders_since\n  )\n  AND users.id IN (/*SLICE:ids*/?) -- :if @ids\nORDER BY\n  users.id ASC, -- :flag @id_asc\n  users.id DESC -- :flag @id_desc\nLIMIT ?",
            &params(&["email", "phone", "orders_since", "ids", "row_limit"]),
            Dialect::MySql,
        )
        .unwrap()
        .unwrap();

        assert!(
            postgres
                .annotated_sql
                .contains("ANY($4::bigint[]) -- :if $4")
        );
        assert!(
            mysql
                .annotated_sql
                .contains("IN (/*SLICE:ids*/?4) -- :if $4")
        );
        assert_eq!(postgres.conditional_param_numbers, [1, 2, 3, 4]);
        assert_eq!(mysql.conditional_param_numbers, [1, 2, 3, 4]);
    }

    #[test]
    fn supports_fully_optional_update_and_delete_where_clauses() {
        for (sql, expected) in [
            (
                "UPDATE users SET status = 'active'\nWHERE\n  email = $1 -- :if @email\n;",
                "UPDATE users SET status = 'active'\n;",
            ),
            (
                "DELETE FROM users\nWHERE\n  email = $1 -- :if @email\n;",
                "DELETE FROM users\n;",
            ),
        ] {
            let info = parse_uncatalogued(sql, &params(&["email"]), Dialect::PostgreSql)
                .unwrap()
                .unwrap();
            let compiled = compile(&info, Placeholders::Numbered, &[1]);

            assert_eq!(compiled.build(&[Arg::Inactive]).0, expected);
        }
    }

    #[test]
    fn ignores_annotations_inside_literals_and_block_comments() {
        for sql in [
            "SELECT $text$\n-- :if @id\n$text$ FROM users WHERE id = $1",
            "SELECT id FROM users\n/*\n-- :if @id\n*/\nWHERE id = $1",
        ] {
            assert!(
                parse_uncatalogued(sql, &params(&["id"]), Dialect::PostgreSql)
                    .unwrap()
                    .is_none()
            );
        }
    }

    #[test]
    fn renders_parenthesized_or_group_as_one_conjunct() {
        let info = parse_uncatalogued(
            "SELECT id FROM users\nWHERE active = $2\n  AND (a = $1 OR b = $1) -- :if @x",
            &params(&["x", "active"]),
            Dialect::PostgreSql,
        )
        .unwrap()
        .unwrap();
        let compiled = compile(&info, Placeholders::Numbered, &[1, 2]);

        assert_eq!(
            compiled.build(&[Arg::Inactive, Arg::Active]).0,
            "SELECT id FROM users\nWHERE\n  active = $1"
        );
        assert_eq!(
            compiled.build(&[Arg::Active, Arg::Active]).0,
            "SELECT id FROM users\nWHERE\n  active = $1\n  AND (a = $2 OR b = $2)"
        );
    }

    #[test]
    fn rejects_parse_failures_duplicate_and_dangling_annotations() {
        for sql in [
            "SELECT FROM users -- :if @id",
            "SELECT id FROM users\n-- :if @id\nWHERE id = $1 -- :if @id",
            "SELECT id FROM users\n-- :if @id\nLIMIT $1",
            "SELECT id FROM users WHERE id = $1 + -- :if @id\n  1",
            "SELECT id FROM users WHERE EXISTS (SELECT id -- :if @id\nFROM users)",
        ] {
            assert!(
                parse_uncatalogued(sql, &params(&["id"]), Dialect::PostgreSql).is_err(),
                "{sql}"
            );
        }
    }

    #[test]
    fn renders_every_search_users_variant() {
        let info = parse_uncatalogued(
            "SELECT id, email, phone\nFROM users\nWHERE\n  email = ?1 -- :if @email\n  AND phone = ?2 -- :if @phone\n  AND EXISTS ( -- :flag @has_orders\n    SELECT 1 FROM orders\n    WHERE orders.user_id = users.id\n      AND orders.created_at >= ?3 -- :if @orders_since\n  )\n  AND users.id IN (/*SLICE:ids*/?4) -- :if @ids\nORDER BY\n  users.id ASC, -- :flag @id_asc\n  users.id DESC -- :flag @id_desc\nLIMIT ?5",
            &params(&["email", "phone", "orders_since", "ids", "row_limit"]),
            Dialect::Sqlite,
        )
        .unwrap()
        .unwrap();
        let compiled = compile(&info, Placeholders::NumberedSqlite, &[1, 2, 3, 4, 5]);
        let inactive = Arg::Inactive;
        let active = Arg::Active;
        let no_filters = [
            inactive,
            inactive,
            inactive,
            Arg::Slice(None),
            active,
            Arg::Flag(false),
            Arg::Flag(false),
            Arg::Flag(false),
        ];
        let cases = [
            (
                no_filters,
                "SELECT id, email, phone\nFROM users\nLIMIT $1",
                vec![Bind::Arg(4)],
            ),
            (
                [
                    active,
                    inactive,
                    inactive,
                    Arg::Slice(None),
                    active,
                    Arg::Flag(false),
                    Arg::Flag(false),
                    Arg::Flag(false),
                ],
                "SELECT id, email, phone\nFROM users\nWHERE\n  email = $1\nLIMIT $2",
                vec![Bind::Arg(0), Bind::Arg(4)],
            ),
            (
                [
                    inactive,
                    active,
                    inactive,
                    Arg::Slice(None),
                    active,
                    Arg::Flag(false),
                    Arg::Flag(false),
                    Arg::Flag(false),
                ],
                "SELECT id, email, phone\nFROM users\nWHERE\n  phone = $1\nLIMIT $2",
                vec![Bind::Arg(1), Bind::Arg(4)],
            ),
            (
                [
                    inactive,
                    inactive,
                    inactive,
                    Arg::Slice(Some(2)),
                    active,
                    Arg::Flag(false),
                    Arg::Flag(false),
                    Arg::Flag(false),
                ],
                "SELECT id, email, phone\nFROM users\nWHERE\n  users.id IN ($1,$2)\nLIMIT $3",
                vec![Bind::Elem(3, 0), Bind::Elem(3, 1), Bind::Arg(4)],
            ),
            (
                [
                    inactive,
                    inactive,
                    inactive,
                    Arg::Slice(None),
                    active,
                    Arg::Flag(true),
                    Arg::Flag(false),
                    Arg::Flag(false),
                ],
                "SELECT id, email, phone\nFROM users\nWHERE\n  EXISTS (\n    SELECT 1 FROM orders\n    WHERE\n      orders.user_id = users.id\n  )\nLIMIT $1",
                vec![Bind::Arg(4)],
            ),
            (
                [
                    inactive,
                    inactive,
                    active,
                    Arg::Slice(None),
                    active,
                    Arg::Flag(true),
                    Arg::Flag(false),
                    Arg::Flag(false),
                ],
                "SELECT id, email, phone\nFROM users\nWHERE\n  EXISTS (\n    SELECT 1 FROM orders\n    WHERE\n      orders.user_id = users.id\n      AND orders.created_at >= $1\n  )\nLIMIT $2",
                vec![Bind::Arg(2), Bind::Arg(4)],
            ),
            (
                [
                    inactive,
                    inactive,
                    active,
                    Arg::Slice(None),
                    active,
                    Arg::Flag(false),
                    Arg::Flag(false),
                    Arg::Flag(false),
                ],
                "SELECT id, email, phone\nFROM users\nLIMIT $1",
                vec![Bind::Arg(4)],
            ),
            (
                [
                    inactive,
                    inactive,
                    inactive,
                    Arg::Slice(None),
                    active,
                    Arg::Flag(false),
                    Arg::Flag(true),
                    Arg::Flag(false),
                ],
                "SELECT id, email, phone\nFROM users\nORDER BY\n  users.id ASC\nLIMIT $1",
                vec![Bind::Arg(4)],
            ),
            (
                [
                    inactive,
                    inactive,
                    inactive,
                    Arg::Slice(None),
                    active,
                    Arg::Flag(false),
                    Arg::Flag(false),
                    Arg::Flag(true),
                ],
                "SELECT id, email, phone\nFROM users\nORDER BY\n  users.id DESC\nLIMIT $1",
                vec![Bind::Arg(4)],
            ),
            (
                [
                    active,
                    active,
                    active,
                    Arg::Slice(Some(2)),
                    active,
                    Arg::Flag(true),
                    Arg::Flag(true),
                    Arg::Flag(true),
                ],
                "SELECT id, email, phone\nFROM users\nWHERE\n  email = $1\n  AND phone = $2\n  AND EXISTS (\n    SELECT 1 FROM orders\n    WHERE\n      orders.user_id = users.id\n      AND orders.created_at >= $3\n  )\n  AND users.id IN ($4,$5)\nORDER BY\n  users.id ASC,\n  users.id DESC\nLIMIT $6",
                vec![
                    Bind::Arg(0),
                    Bind::Arg(1),
                    Bind::Arg(2),
                    Bind::Elem(3, 0),
                    Bind::Elem(3, 1),
                    Bind::Arg(4),
                ],
            ),
        ];

        for (args, sql, binds) in cases {
            assert_eq!(compiled.build(&args), (sql.into(), binds));
        }
    }
}

mod switches {
    use crate::dynfilter::test_support::{compile, params, parse_uncatalogued};
    use crate::dynfilter::{Dialect, DynFilterInfo, Switch};
    use crate::dynfilter_runtime::{Arg, Bind, Placeholders};

    const SEARCH_USERS: &str = "SELECT id, email, phone\nFROM users\nWHERE\n  email = ?1 -- :if @email\n  AND EXISTS ( -- :flag @has_orders\n    SELECT 1 FROM orders WHERE orders.user_id = users.id\n  )\nORDER BY -- :switch @sort id_asc id_desc shortest_email default=id_asc\n  users.id ASC,    -- :case @id_asc\n  users.id DESC,   -- :case @id_desc\n  LENGTH(users.email) ASC, users.id DESC -- :case @shortest_email\nLIMIT ?2";

    fn search_users() -> DynFilterInfo {
        parse_uncatalogued(
            SEARCH_USERS,
            &params(&["email", "row_limit"]),
            Dialect::Sqlite,
        )
        .unwrap()
        .unwrap()
    }

    #[test]
    fn attaches_an_inline_switch_to_order_by() {
        let info = search_users();

        assert_eq!(
            info.switches,
            [Switch {
                field: "sort".into(),
                choices: vec!["id_asc".into(), "id_desc".into(), "shortest_email".into()],
                default: "id_asc".into(),
            }]
        );
        assert_eq!(
            info.flag_params
                .iter()
                .map(|flag| (flag.name.as_str(), flag.switch))
                .collect::<Vec<_>>(),
            [
                ("has_orders", None),
                ("id_asc", Some(0)),
                ("id_desc", Some(0)),
                ("shortest_email", Some(0))
            ]
        );
        assert_eq!(
            info.annotated_sql,
            "SELECT id, email, phone\nFROM users\nWHERE\n  email = ?1 -- :if $1\n  AND EXISTS ( -- :if $3\n    SELECT 1 FROM orders WHERE orders.user_id = users.id -- :if $3\n  ) -- :if $3\nORDER BY\n  users.id ASC, -- :if $4\n  users.id DESC, -- :if $5\n  LENGTH(users.email) ASC, -- :if $6\n  users.id DESC -- :if $6\nLIMIT ?2"
        );
    }

    #[test]
    fn renders_every_search_users_preset() {
        let info = search_users();
        let compiled = compile(&info, Placeholders::NumberedSqlite, &[1, 2]);
        let preset = |index: usize| {
            [
                Arg::Inactive,
                Arg::Active,
                Arg::Flag(false),
                Arg::Flag(index == 0),
                Arg::Flag(index == 1),
                Arg::Flag(index == 2),
            ]
        };

        for (args, sql) in [
            (
                preset(0),
                "SELECT id, email, phone\nFROM users\nORDER BY\n  users.id ASC\nLIMIT $1",
            ),
            (
                preset(1),
                "SELECT id, email, phone\nFROM users\nORDER BY\n  users.id DESC\nLIMIT $1",
            ),
            (
                preset(2),
                "SELECT id, email, phone\nFROM users\nORDER BY\n  LENGTH(users.email) ASC,\n  users.id DESC\nLIMIT $1",
            ),
        ] {
            assert_eq!(compiled.build(&args), (sql.into(), vec![Bind::Arg(1)]));
        }
    }

    #[test]
    fn attaches_a_standalone_switch_to_where_and_renders_each_scope() {
        let info = parse_uncatalogued(
            "SELECT id FROM notes\n-- :switch @scope mine all default=mine\nWHERE\n  owner_id = $1 -- :case @mine\n  AND TRUE -- :case @all\n  AND archived = $2 -- :if @archived\nORDER BY id",
            &params(&["owner_id", "archived"]),
            Dialect::PostgreSql,
        )
        .unwrap()
        .unwrap();
        let compiled = compile(&info, Placeholders::Numbered, &[1, 2]);

        assert_eq!(info.switches[0].field, "scope");
        assert!(!info.conditional_param_numbers.contains(&1));
        assert_eq!(
            compiled.build(&[
                Arg::Active,
                Arg::Inactive,
                Arg::Flag(true),
                Arg::Flag(false)
            ]),
            (
                "SELECT id FROM notes\nWHERE\n  owner_id = $1\nORDER BY id".into(),
                vec![Bind::Arg(0)]
            )
        );
        assert_eq!(
            compiled.build(&[Arg::Active, Arg::Active, Arg::Flag(false), Arg::Flag(true)]),
            (
                "SELECT id FROM notes\nWHERE\n  TRUE\n  AND archived = $1\nORDER BY id".into(),
                vec![Bind::Arg(1)]
            )
        );
    }

    #[test]
    fn accepts_case_less_tie_breakers_and_switches_on_two_clauses() {
        let info = parse_uncatalogued(
            "SELECT id FROM t\nWHERE -- :switch @scope mine all default=all\n  owner_id = $1 -- :case @mine\n  AND TRUE -- :case @all\n  AND deleted = FALSE\n-- :switch @sort newest oldest default=newest\nORDER BY\n  created_at DESC, -- :case @newest\n  created_at ASC, -- :case @oldest\n  id ASC",
            &params(&["owner_id"]),
            Dialect::PostgreSql,
        )
        .unwrap()
        .unwrap();

        assert_eq!(info.switches.len(), 2);
        assert!(info.annotated_sql.contains("AND deleted = FALSE\n"));
        assert!(info.annotated_sql.ends_with("  id ASC"));
        assert!(!info.annotated_sql.contains(":switch"));
    }

    #[test]
    fn keeps_annotation_lookalikes_inside_literals() {
        let info = parse_uncatalogued(
            "SELECT id FROM t\nWHERE note = $1 -- :if @note\n  AND kind = '-- :x' -- :if @kind",
            &params(&["note", "kind"]),
            Dialect::PostgreSql,
        )
        .unwrap()
        .unwrap();

        assert_eq!(
            info.annotated_sql,
            "SELECT id FROM t\nWHERE\n  note = $1 -- :if $1\n  AND kind = '-- :x' -- :if $2"
        );
    }

    #[test]
    fn rejects_invalid_switch_and_case_placement() {
        for (sql, expected) in [
            (
                "SELECT id FROM t WHERE a = $1 -- :switch @s x y default=x\n  AND b = $1 -- :case @x\n  AND c = $1 -- :case @y",
                "must follow a WHERE or ORDER BY keyword",
            ),
            (
                "SELECT id -- :switch @s x y default=x\nFROM t WHERE a = $1 -- :case @x\n  AND b = $1 -- :case @y",
                "must follow a WHERE or ORDER BY keyword",
            ),
            (
                "SELECT id FROM t\n-- :switch @s x y default=x\nWHERE -- :switch @t p q default=p\n  a = $1 -- :case @x\n  AND b = $1 -- :case @y",
                "second switch on one WHERE clause",
            ),
            (
                "SELECT id FROM t WHERE a = $1 -- :case @x",
                "is in a WHERE clause without a `-- :switch`",
            ),
            (
                "SELECT id FROM t\nWHERE -- :switch @s x y default=x\n  a = $1 -- :case @x\n  AND b = $1 -- :case @z",
                "names a choice not declared by `-- :switch @s`",
            ),
            (
                "SELECT id FROM t\nWHERE -- :switch @s x y default=x\n  a = $1 -- :case @x\n  AND b = $1",
                "declares choice `y` but its WHERE clause has no `-- :case @y`",
            ),
            (
                "SELECT id FROM t\nWHERE -- :switch @s x y default=x\n  -- :case @x\n  a = $1 -- :case @y",
                "a structure takes one annotation",
            ),
            (
                "SELECT id FROM t\nWHERE -- :switch @s x y default=x\n  -- :flag @f\n  a = $1 -- :case @x\n  AND b = $1 -- :case @y",
                "a structure takes one annotation",
            ),
            (
                "SELECT id FROM t\nWHERE -- :switch @s x y default=x\n  -- :if @id\n  a = $1 -- :case @x\n  AND b = $1 -- :case @y",
                "a structure takes one annotation",
            ),
            (
                "SELECT id FROM t\nWHERE -- :switch @s x y default=x\n  a = $1 -- :case @x\n  AND b = $1 -- :case @y\n  AND EXISTS (SELECT 1 FROM u WHERE u.t = t.id -- :case @x\n  )",
                "is in a WHERE clause without a `-- :switch`",
            ),
            (
                "SELECT id FROM t WHERE a = $1 -- :sort @x",
                "unknown annotation `-- :sort`",
            ),
            (
                "SELECT id FROM t WHERE a = $1 -- :if @missing",
                "use `-- :flag @missing`",
            ),
            (
                "SELECT id FROM t WHERE a = $1 -- :flag @id",
                "use `-- :if @id`",
            ),
        ] {
            let error = parse_uncatalogued(sql, &params(&["id"]), Dialect::PostgreSql).unwrap_err();
            assert!(error.contains(expected), "{sql}\n{error}");
        }
    }

    #[test]
    fn inline_annotation_covers_sibling_structures_on_its_line() {
        let info = parse_uncatalogued(
            "SELECT id FROM users\nWHERE\n  email = $1 AND phone = $2 -- :if @email\n  AND EXISTS (SELECT 1 FROM orders WHERE orders.user_id = users.id AND orders.paid = $3) -- :flag @has_orders\nORDER BY id",
            &params(&["email", "phone", "paid"]),
            Dialect::PostgreSql,
        )
        .unwrap()
        .unwrap();

        assert_eq!(
            info.annotated_sql,
            "SELECT id FROM users\nWHERE\n  email = $1 -- :if $1\n  AND phone = $2 -- :if $1\n  AND EXISTS (SELECT 1 FROM orders WHERE orders.user_id = users.id AND orders.paid = $3) -- :if $4\nORDER BY id"
        );
        let compiled = compile(&info, Placeholders::Numbered, &[1, 2, 3]);
        assert_eq!(
            compiled
                .build(&[Arg::Inactive, Arg::Active, Arg::Active, Arg::Flag(false)])
                .0,
            "SELECT id FROM users\nORDER BY id"
        );
        assert_eq!(
            compiled
                .build(&[Arg::Active, Arg::Active, Arg::Active, Arg::Flag(true)])
                .0,
            "SELECT id FROM users\nWHERE\n  email = $1\n  AND phone = $2\n  AND EXISTS (SELECT 1 FROM orders WHERE orders.user_id = users.id AND orders.paid = $3)\nORDER BY id"
        );
    }

    #[test]
    fn inline_annotation_rejects_a_sibling_already_annotated() {
        let error = parse_uncatalogued(
            "SELECT id FROM users\nWHERE\n  -- :flag @first\n  email = $1 AND phone = $2 -- :if @phone\nORDER BY id",
            &params(&["email", "phone"]),
            Dialect::PostgreSql,
        )
        .unwrap_err();

        assert!(
            error.contains("already annotated by `-- :flag` on line 3"),
            "{error}"
        );
    }

    #[test]
    fn duplicate_parameter_names_do_not_shift_control_slots() {
        let info = parse_uncatalogued(
            "SELECT id FROM t\nWHERE\n  id > ?1\n  AND id < ?2\n  AND archived = 0 -- :flag @hide_archived\nORDER BY id",
            &params(&["id", "id"]),
            Dialect::Sqlite,
        )
        .unwrap()
        .unwrap();
        assert!(info.annotated_sql.contains("archived = 0 -- :if $3"));
        let compiled = compile(&info, Placeholders::NumberedSqlite, &[1, 2]);

        assert_eq!(
            compiled.build(&[Arg::Active, Arg::Active, Arg::Flag(false)]),
            (
                "SELECT id FROM t\nWHERE\n  id > $1\n  AND id < $2\nORDER BY id".into(),
                vec![Bind::Arg(0), Bind::Arg(1)]
            )
        );
    }

    #[test]
    fn non_ascii_comments_do_not_break_the_runtime() {
        let info = parse_uncatalogued(
            "SELECT id FROM t -- café ☕\nWHERE id = 1 -- :flag @enabled\nORDER BY id",
            &params(&[]),
            Dialect::PostgreSql,
        )
        .unwrap()
        .unwrap();
        let compiled = compile(&info, Placeholders::Numbered, &[]);

        assert_eq!(
            compiled.build(&[Arg::Flag(false)]).0,
            "SELECT id FROM t\nORDER BY id"
        );
    }

    #[test]
    fn uses_dialect_string_rules_when_finding_annotations() {
        for (dialect, sql) in [
            (
                Dialect::Sqlite,
                "SELECT id FROM t\nWHERE path = '\\' -- :flag @enabled\nORDER BY id",
            ),
            (
                Dialect::PostgreSql,
                "SELECT id FROM t\nWHERE path = '\\' -- :flag @enabled\nORDER BY id",
            ),
            (
                Dialect::MySql,
                "SELECT id FROM t\nWHERE path = 'it\\'s' -- :flag @enabled\nORDER BY id",
            ),
            (
                Dialect::PostgreSql,
                "SELECT id FROM t\nWHERE path = E'\\'' -- :flag @enabled\nORDER BY id",
            ),
        ] {
            let info = parse_uncatalogued(sql, &params(&[]), dialect)
                .unwrap()
                .unwrap();
            let placeholders = match dialect {
                Dialect::MySql => Placeholders::Question,
                Dialect::Sqlite => Placeholders::NumberedSqlite,
                Dialect::PostgreSql => Placeholders::Numbered,
            };
            let compiled = compile(&info, placeholders, &[]);
            let predicate = sql.lines().nth(1).unwrap().split(" -- ").next().unwrap();
            let predicate = predicate.trim().strip_prefix("WHERE ").unwrap();
            assert_eq!(
                compiled.build(&[Arg::Flag(true)]).0,
                format!("SELECT id FROM t\nWHERE\n  {predicate}\nORDER BY id"),
                "{sql}"
            );
            assert_eq!(
                compiled.build(&[Arg::Flag(false)]).0,
                "SELECT id FROM t\nORDER BY id",
                "{sql}"
            );
        }
    }

    #[test]
    fn finds_annotations_after_a_multiline_comment_closes() {
        let info = parse_uncatalogued(
            "SELECT id FROM t\nWHERE /* first\nlast */ id = 1 -- :flag @enabled\nORDER BY id",
            &params(&[]),
            Dialect::PostgreSql,
        )
        .unwrap()
        .unwrap();
        let compiled = compile(&info, Placeholders::Numbered, &[]);

        assert_eq!(
            compiled.build(&[Arg::Flag(false)]).0,
            "SELECT id FROM t\nORDER BY id"
        );
        // The blanked comment leaves its width as indentation; the SQL stays valid.
        assert_eq!(
            compiled.build(&[Arg::Flag(true)]).0,
            "SELECT id FROM t\nWHERE\n        id = 1\nORDER BY id"
        );

        let error = parse_uncatalogued(
            "SELECT id FROM t\nWHERE note = 'first\nlast' -- :sort @x\nORDER BY id",
            &params(&[]),
            Dialect::PostgreSql,
        )
        .unwrap_err();
        assert!(error.contains("unknown annotation `-- :sort`"), "{error}");
    }

    /// An ungated literal spanning lines is data: blank lines, trailing spaces, and text that
    /// looks like a placeholder or directive survive canonicalization and every rendered state.
    #[test]
    fn preserves_ungated_multiline_literals_byte_for_byte() {
        let literal = "'first  \n\n  $2 ?1 -- :if @id\nlast'";
        let sql = format!(
            "SELECT id, {literal} AS label\nFROM users\nWHERE\n  id = $1 -- :if @id\n  AND phone <> ''  "
        );
        let info = parse_uncatalogued(&sql, &params(&["id"]), Dialect::PostgreSql)
            .unwrap()
            .unwrap();
        assert!(
            info.annotated_sql.contains(literal),
            "{}",
            info.annotated_sql
        );
        assert!(
            info.annotated_sql.ends_with("AND phone <> ''"),
            "{}",
            info.annotated_sql
        );
        let compiled = compile(&info, Placeholders::Numbered, &[1]);
        let (inactive, binds) = compiled.build(&[Arg::Inactive]);
        assert_eq!(
            inactive,
            format!("SELECT id, {literal} AS label\nFROM users\nWHERE\n  phone <> ''")
        );
        assert!(binds.is_empty());
        let (active, binds) = compiled.build(&[Arg::Active]);
        assert_eq!(
            active,
            format!(
                "SELECT id, {literal} AS label\nFROM users\nWHERE\n  id = $1\n  AND phone <> ''"
            )
        );
        assert_eq!(binds, [Bind::Arg(0)]);

        let dollar = "$tag$one  \n\n   two$tag$";
        let sql = format!("SELECT {dollar} AS label FROM users WHERE id = $1 -- :if @id");
        let info = parse_uncatalogued(&sql, &params(&["id"]), Dialect::PostgreSql)
            .unwrap()
            .unwrap();
        assert!(
            info.annotated_sql.contains(dollar),
            "{}",
            info.annotated_sql
        );
        let compiled = compile(&info, Placeholders::Numbered, &[1]);
        assert_eq!(
            compiled.build(&[Arg::Inactive]).0,
            format!("SELECT {dollar} AS label FROM users")
        );

        let quoted = "\"odd  \n\n name\"";
        let sql = format!("SELECT id AS {quoted} FROM users WHERE id = ?1 -- :if @id");
        let info = parse_uncatalogued(&sql, &params(&["id"]), Dialect::Sqlite)
            .unwrap()
            .unwrap();
        assert!(
            info.annotated_sql.contains(quoted),
            "{}",
            info.annotated_sql
        );
    }

    #[test]
    fn rejects_gated_structures_containing_multiline_values() {
        for sql in [
            "SELECT id FROM t\nWHERE\n  -- :flag @enabled\n  note = 'first\nlast'\nORDER BY id",
            "SELECT id FROM t\nWHERE note = 'first\nlast' -- :flag @enabled\nORDER BY id",
            "SELECT id FROM t\nWHERE note = $q$first\nlast$q$ -- :flag @enabled\nORDER BY id",
        ] {
            let error = parse_uncatalogued(sql, &params(&[]), Dialect::PostgreSql).unwrap_err();
            assert!(
                error.contains("multi-line literal or comment"),
                "{sql}: {error}"
            );
        }
    }

    /// Prose is dropped before markers are appended, so a comment quoting a marker never
    /// gates its line, while block comments stay in the text as inert bytes.
    #[test]
    fn comment_prose_quoting_a_marker_never_gates_a_line() {
        let info = parse_uncatalogued(
            "SELECT id -- Documentation example: -- :if $1\nFROM t /* -- :if $1 */\nWHERE\n  -- filter by email\n  email = $1 -- :if @email\nORDER BY id",
            &params(&["email"]),
            Dialect::PostgreSql,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            info.annotated_sql,
            "SELECT id\nFROM t /* -- :if $1 */\nWHERE\n  email = $1 -- :if $1\nORDER BY id"
        );
        let compiled = compile(&info, Placeholders::Numbered, &[1]);
        assert_eq!(
            compiled.build(&[Arg::Inactive]),
            (
                "SELECT id\nFROM t /* -- :if $1 */\nORDER BY id".to_string(),
                vec![]
            )
        );
        assert_eq!(
            compiled.build(&[Arg::Active]),
            (
                "SELECT id\nFROM t /* -- :if $1 */\nWHERE\n  email = $1\nORDER BY id".to_string(),
                vec![Bind::Arg(0)]
            )
        );
    }

    #[test]
    fn ordinary_comments_mentioning_directives_are_inert() {
        for sql in [
            "SELECT id FROM t\nWHERE id = 1 -- documentation mentions -- :flag @enabled\nORDER BY id",
            "SELECT id FROM t -- example: -- :sort @s\nWHERE id = 1\nORDER BY id",
            "SELECT id FROM t\nWHERE note = '-- :x'\nORDER BY id",
        ] {
            assert!(
                parse_uncatalogued(sql, &params(&[]), Dialect::PostgreSql)
                    .unwrap()
                    .is_none(),
                "{sql}"
            );
        }
        let error = parse_uncatalogued(
            "SELECT id FROM t\nWHERE a = $1 -- :if @a -- :flag @b\nORDER BY id",
            &params(&["a"]),
            Dialect::PostgreSql,
        )
        .unwrap_err();
        assert!(
            error.contains("`-- :if` expects one or more `@name`"),
            "{error}"
        );
    }

    #[test]
    fn rejects_optional_parameters_bound_outside_their_guard() {
        let error = parse_uncatalogued(
            "SELECT id FROM t\nWHERE id = ?1 -- :if @id\nORDER BY id\nLIMIT ?1",
            &params(&["id"]),
            Dialect::Sqlite,
        )
        .unwrap_err();
        assert!(
            error.contains("optional parameter `id`") && error.contains("line 4"),
            "{error}"
        );

        let error = parse_uncatalogued(
            "SELECT id FROM t\nWHERE\n  owner = $1 -- :if @owner\nORDER BY -- :switch @sort mine all default=all\n  owner ASC, -- :case @mine\n  id ASC -- :case @all\nLIMIT $1",
            &params(&["owner"]),
            Dialect::PostgreSql,
        )
        .unwrap_err();
        assert!(error.contains("optional parameter `owner`"), "{error}");

        let info = parse_uncatalogued(
            "SELECT id FROM t\nWHERE\n  a = $1 -- :if @a\n  AND b = $1 -- :if @a\n  AND EXISTS ( -- :flag @has\n    SELECT 1 FROM o WHERE o.x = $2 -- :if @x\n  )\nORDER BY id",
            &params(&["a", "x"]),
            Dialect::PostgreSql,
        )
        .unwrap()
        .unwrap();
        assert_eq!(info.conditional_param_numbers, [1, 2]);

        let info = parse_uncatalogued(
            "SELECT id FROM t\nWHERE\n  email = ? -- :if @email\n  AND id IN (/*SLICE:ids*/?) -- :if @ids\nLIMIT ?",
            &[("email".into(), 1), ("ids".into(), 3), ("row_limit".into(), 2)],
            Dialect::MySql,
        )
        .unwrap()
        .unwrap();
        assert_eq!(info.conditional_param_numbers, [1, 3]);
    }
}

mod joins {
    use crate::dynfilter::Dialect;
    use crate::dynfilter::test_support::{
        compile, params, parse_catalogued as parse_with_catalog, parse_uncatalogued,
    };
    use crate::dynfilter_runtime::{Arg, Bind, Placeholders};

    #[test]
    fn gates_inline_and_standalone_joins_in_each_dialect() {
        for (dialect, placeholders, since, limit, out_since, out_limit) in [
            (
                Dialect::PostgreSql,
                Placeholders::Numbered,
                "$1",
                "$2",
                "$1",
                "$2",
            ),
            (
                Dialect::Sqlite,
                Placeholders::NumberedSqlite,
                "?1",
                "?2",
                "$1",
                "$2",
            ),
            (Dialect::MySql, Placeholders::Question, "?", "?", "?", "?"),
        ] {
            let sql = format!(
                "SELECT u.id, u.email\nFROM users u\nLEFT JOIN orders o ON o.user_id = u.id -- :flag @with_orders\n-- :if @since\nJOIN payments p\n  ON p.user_id = u.id\n  AND p.paid_at >= {since}\nWHERE\n  o.id IS NOT NULL -- :flag @with_orders\nORDER BY\n  p.paid_at DESC, -- :if @since\n  u.id ASC\nLIMIT {limit}"
            );
            let info = parse_with_catalog(&sql, &params(&["since", "row_limit"]), dialect)
                .unwrap()
                .unwrap();
            assert!(
                info.annotated_sql.contains(
                    "LEFT JOIN orders o ON o.user_id = u.id -- :if $3\nJOIN payments p -- :if $1\n  ON p.user_id = u.id -- :if $1\n  AND p.paid_at >= "
                ),
                "{}",
                info.annotated_sql
            );
            let compiled = compile(&info, placeholders, &[1, 2]);

            let (off, binds) = compiled.build(&[Arg::Inactive, Arg::Active, Arg::Flag(false)]);
            assert_eq!(
                off,
                format!(
                    "SELECT u.id, u.email\nFROM users u\nORDER BY\n  u.id ASC\nLIMIT {out_since}"
                )
            );
            assert_eq!(binds, [Bind::Arg(1)]);

            let (on, binds) = compiled.build(&[Arg::Active, Arg::Active, Arg::Flag(true)]);
            assert_eq!(
                on,
                format!(
                    "SELECT u.id, u.email\nFROM users u\nLEFT JOIN orders o ON o.user_id = u.id\nJOIN payments p\n  ON p.user_id = u.id\n  AND p.paid_at >= {out_since}\nWHERE\n  o.id IS NOT NULL\nORDER BY\n  p.paid_at DESC,\n  u.id ASC\nLIMIT {out_limit}"
                )
            );
            assert_eq!(binds, [Bind::Arg(0), Bind::Arg(1)]);
        }
    }

    #[test]
    fn accepts_references_under_the_same_gate() {
        let info = parse_with_catalog(
            "SELECT u.id\nFROM users u\nJOIN orders o ON o.user_id = u.id -- :flag @with_orders @recent\nWHERE\n  u.id > 0\n  AND EXISTS ( -- :flag @with_orders @recent\n    SELECT 1 FROM items i WHERE i.order_id = o.id\n  )\n  AND o.created_at > '2024' -- :flag @recent @with_orders\nORDER BY\n  o.id DESC, -- :flag @with_orders @recent\n  u.id ASC",
            &params(&[]),
            Dialect::PostgreSql,
        )
        .unwrap()
        .unwrap();
        assert_eq!(info.flag_params.len(), 2);
    }

    #[test]
    fn resolves_unqualified_columns_by_scope_and_catalog() {
        for sql in [
            "SELECT u.id, u.email AS created_at\nFROM users u\nJOIN orders o ON o.user_id = u.id -- :flag @with_orders\nWHERE u.id > 0",
            "SELECT u.id\nFROM users u\nJOIN orders o ON o.user_id = u.id -- :flag @with_orders\nORDER BY \"Created_At\"",
            "SELECT u.id\nFROM users u\nJOIN orders o ON o.user_id = u.id -- :flag @with_orders\nWHERE EXISTS (SELECT 1 FROM orders WHERE created_at > '2024')",
            "SELECT u.id\nFROM users u\nJOIN orders o ON o.user_id = u.id -- :flag @with_orders\nWHERE email <> ''",
        ] {
            let result = parse_with_catalog(sql, &params(&[]), Dialect::PostgreSql);
            assert!(result.is_ok(), "{sql}: {result:?}");
        }
        for (sql, expected) in [
            (
                "SELECT u.id\nFROM users u\nJOIN orders o ON o.user_id = u.id -- :flag @with_orders\nWHERE EXISTS (SELECT 1 FROM audits WHERE created_at > '2024')",
                "column `created_at` on line 4 cannot be resolved because it may belong to table `audits`, which the sqlc catalog does not describe",
            ),
            (
                "SELECT u.id\nFROM users u\nJOIN audits a ON a.user_id = u.id -- :flag @with_audits\nWHERE created_at > '2024'",
                "column `created_at` on line 4 cannot be resolved because it may belong to table `audits`, which the sqlc catalog does not describe",
            ),
            (
                "SELECT u.id\nFROM users u\nJOIN audits a ON a.user_id = u.id -- :flag @with_audits\nWHERE EXISTS (SELECT 1 FROM orders WHERE user_id = u.id AND note = '')",
                "column `note` on line 4 cannot be resolved",
            ),
        ] {
            let error = parse_with_catalog(sql, &params(&[]), Dialect::PostgreSql).unwrap_err();
            assert!(error.contains(expected), "{sql}\n{error}");
        }
    }

    #[test]
    fn rejects_unsound_gated_joins() {
        for (sql, expected) in [
            (
                "SELECT u.id, o.id\nFROM users u\nJOIN orders o ON o.user_id = u.id -- :flag @x",
                "alias `o` is referenced on line 1",
            ),
            (
                "SELECT u.id\nFROM users u\nJOIN orders o ON o.user_id = u.id -- :flag @x\nWHERE o.id > 0",
                "alias `o` is referenced on line 4",
            ),
            (
                "SELECT u.id\nFROM users u\nJOIN orders o ON o.user_id = u.id -- :flag @x\nJOIN items i ON i.order_id = o.id",
                "alias `o` is referenced on line 4",
            ),
            (
                "SELECT u.id\nFROM users u\nJOIN orders o ON o.user_id = u.id -- :flag @x\nWHERE EXISTS (SELECT 1 FROM items i WHERE i.order_id = o.id)",
                "alias `o` is referenced on line 4",
            ),
            (
                "SELECT u.id\nFROM users u\nJOIN orders o ON o.user_id = u.id -- :flag @x\nWHERE o.id > 0 -- :flag @y",
                "gated by `-- :flag @x`",
            ),
            (
                "SELECT u.id\nFROM users u\nJOIN orders o ON o.user_id = u.id -- :flag @x @y\nWHERE o.id > 0 -- :flag @x",
                "gated by `-- :flag @x @y`",
            ),
            (
                "SELECT *\nFROM users u\nJOIN orders o ON o.user_id = u.id -- :flag @x",
                "SELECT *",
            ),
            (
                "SELECT u.id\nFROM users u\nJOIN orders o ON o.user_id = u.id -- :flag @x\nWHERE created_at > '2024'",
                "column `created_at` of `o` is referenced unqualified on line 4",
            ),
            (
                "SELECT u.id\nFROM users u\nJOIN orders o ON o.user_id = u.id -- :flag @x\nORDER BY Created_At",
                "unqualified",
            ),
            (
                "SELECT u.id\nFROM users u\nJOIN orders o ON o.user_id = u.id -- :flag @x\nORDER BY \"created_at\"",
                "unqualified",
            ),
            (
                "SELECT u.id\nFROM users u JOIN orders o ON o.user_id = u.id -- :flag @x",
                "shares a line",
            ),
            (
                "SELECT u.id\nFROM users u\nJOIN orders o ON o.user_id = u.id JOIN items i ON i.order_id = o.id -- :flag @x",
                "shares a line",
            ),
            (
                "SELECT u.id\nFROM users u\nJOIN orders o USING (user_id) -- :flag @x",
                "does not attach",
            ),
            (
                "SELECT u.id\nFROM users u\nNATURAL JOIN orders o -- :flag @x",
                "NATURAL",
            ),
            (
                "SELECT u.id\nFROM users u\nCROSS JOIN orders o -- :flag @x",
                "only JOIN, INNER JOIN, LEFT JOIN, and LEFT OUTER JOIN",
            ),
            (
                "SELECT u.id\nFROM users u\nRIGHT JOIN orders o ON o.user_id = u.id -- :flag @x",
                "only JOIN, INNER JOIN, LEFT JOIN, and LEFT OUTER JOIN",
            ),
            (
                "SELECT u.id\nFROM users u\nJOIN orders o ON o.user_id = u.id -- :case @x",
                "targets a JOIN",
            ),
        ] {
            let error = parse_with_catalog(sql, &params(&[]), Dialect::PostgreSql).unwrap_err();
            assert!(error.contains(expected), "{sql}\n{error}");
        }
    }

    #[test]
    fn extends_the_join_over_nested_closing_parens() {
        let info = parse_with_catalog(
            "SELECT u.id\nFROM users u\nJOIN orders o ON (o.user_id = u.id AND (o.id > 0)) -- :flag @x\nWHERE u.id > 0",
            &params(&[]),
            Dialect::PostgreSql,
        )
        .unwrap()
        .unwrap();
        let compiled = compile(&info, Placeholders::Numbered, &[]);
        assert_eq!(
            compiled.build(&[Arg::Flag(false)]).0,
            "SELECT u.id\nFROM users u\nWHERE u.id > 0"
        );
        assert_eq!(
            compiled.build(&[Arg::Flag(true)]).0,
            "SELECT u.id\nFROM users u\nJOIN orders o ON (o.user_id = u.id AND (o.id > 0))\nWHERE u.id > 0"
        );
    }

    #[test]
    fn nests_a_gated_where_inside_a_gated_join_subquery() {
        let info = parse_with_catalog(
            "SELECT u.id\nFROM users u\n-- :flag @x\nJOIN orders o\n  ON o.user_id = u.id\n  AND EXISTS (\n    SELECT 1 FROM items i\n    WHERE i.order_id = o.id\n      AND i.sku = $1 -- :if @sku\n  )\nWHERE u.id > 0",
            &params(&["sku"]),
            Dialect::PostgreSql,
        )
        .unwrap()
        .unwrap();
        assert!(
            info.annotated_sql
                .contains("    WHERE -- :if $2\n      i.order_id = o.id -- :if $2\n      AND i.sku = $1 -- :if $1 -- :if $2"),
            "{}",
            info.annotated_sql
        );
        let compiled = compile(&info, Placeholders::Numbered, &[1]);
        assert_eq!(
            compiled.build(&[Arg::Inactive, Arg::Flag(false)]).0,
            "SELECT u.id\nFROM users u\nWHERE u.id > 0"
        );
        assert_eq!(
            compiled.build(&[Arg::Inactive, Arg::Flag(true)]).0,
            "SELECT u.id\nFROM users u\nJOIN orders o\n  ON o.user_id = u.id\n  AND EXISTS (\n    SELECT 1 FROM items i\n    WHERE\n      i.order_id = o.id\n  )\nWHERE u.id > 0"
        );
        let (sql, binds) = compiled.build(&[Arg::Active, Arg::Flag(true)]);
        assert_eq!(
            sql,
            "SELECT u.id\nFROM users u\nJOIN orders o\n  ON o.user_id = u.id\n  AND EXISTS (\n    SELECT 1 FROM items i\n    WHERE\n      i.order_id = o.id\n      AND i.sku = $1\n  )\nWHERE u.id > 0"
        );
        assert_eq!(binds, [Bind::Arg(0)]);
    }

    #[test]
    fn gates_a_join_whose_bind_lives_in_a_where_conjunct() {
        let info = parse_uncatalogued(
            "SELECT a.id\nFROM assets a\nJOIN owners w ON w.asset_id = a.id -- :if @owner_ids\nWHERE\n  a.deleted = 0\n  AND w.owner_id IN (/*SLICE:owner_ids*/?1) -- :if @owner_ids\nORDER BY a.id",
            &params(&["owner_ids"]),
            Dialect::Sqlite,
        )
        .unwrap()
        .unwrap();
        let compiled = compile(&info, Placeholders::NumberedSqlite, &[1]);
        assert_eq!(
            compiled.build(&[Arg::Slice(None)]).0,
            "SELECT a.id\nFROM assets a\nWHERE\n  a.deleted = 0\nORDER BY a.id"
        );
        let (sql, binds) = compiled.build(&[Arg::Slice(Some(2))]);
        assert_eq!(
            sql,
            "SELECT a.id\nFROM assets a\nJOIN owners w ON w.asset_id = a.id\nWHERE\n  a.deleted = 0\n  AND w.owner_id IN ($1,$2)\nORDER BY a.id"
        );
        assert_eq!(binds, [Bind::Elem(0, 0), Bind::Elem(0, 1)]);
    }

    #[test]
    fn compares_quoted_aliases_exactly() {
        let error = parse_with_catalog(
            "SELECT u.id\nFROM users u\nJOIN orders \"O\" ON \"O\".user_id = u.id -- :flag @x\nWHERE \"O\".id > 0",
            &params(&[]),
            Dialect::PostgreSql,
        )
        .unwrap_err();
        assert!(
            error.contains("alias `O` is referenced on line 4"),
            "{error}"
        );
        assert!(
            parse_with_catalog(
                "SELECT u.id\nFROM users u\nJOIN orders \"O\" ON \"O\".user_id = u.id -- :flag @x\nWHERE u.id > 0 -- :flag @y\n  AND \"O\".id > 0 -- :flag @x",
                &params(&[]),
                Dialect::PostgreSql,
            )
            .is_ok()
        );
    }

    #[test]
    fn rejects_a_gated_join_whose_alias_is_referenced_in_an_ungated_join() {
        let error = parse_with_catalog(
            "SELECT u.id\nFROM users u\nJOIN orders o ON o.user_id = u.id -- :flag @x\nLEFT JOIN items i ON i.order_id = o.id -- :flag @y",
            &params(&[]),
            Dialect::PostgreSql,
        )
        .unwrap_err();
        assert!(
            error.contains("alias `o` is referenced on line 4"),
            "{error}"
        );
    }

    #[test]
    fn renders_consumer_join_shapes_for_sqlite() {
        let info = parse_uncatalogued(
            "SELECT a.id\nFROM assets a\nJOIN tags t ON t.asset_id = a.id AND t.name IN (/*SLICE:tags*/?1) -- :if @tags\nJOIN labels l ON l.asset_id = a.id AND l.name IN (/*SLICE:labels*/?2) -- :if @labels\nWHERE a.deleted = 0\nORDER BY a.id",
            &params(&["tags", "labels"]),
            Dialect::Sqlite,
        )
        .unwrap()
        .unwrap();
        let compiled = compile(&info, Placeholders::NumberedSqlite, &[1, 2]);
        assert_eq!(
            compiled.build(&[Arg::Slice(None), Arg::Slice(None)]).0,
            "SELECT a.id\nFROM assets a\nWHERE a.deleted = 0\nORDER BY a.id"
        );
        let (sql, binds) = compiled.build(&[Arg::Slice(None), Arg::Slice(Some(2))]);
        assert_eq!(
            sql,
            "SELECT a.id\nFROM assets a\nJOIN labels l ON l.asset_id = a.id AND l.name IN ($1,$2)\nWHERE a.deleted = 0\nORDER BY a.id"
        );
        assert_eq!(binds, [Bind::Elem(1, 0), Bind::Elem(1, 1)]);

        let info = parse_uncatalogued(
            "SELECT a.id\nFROM assets a\nLEFT JOIN reviews r ON r.asset_id = a.id -- :flag @classified_only\nWHERE\n  a.deleted = 0\n  AND r.asset_id IS NOT NULL -- :flag @classified_only\nORDER BY a.id",
            &params(&[]),
            Dialect::Sqlite,
        )
        .unwrap()
        .unwrap();
        let compiled = compile(&info, Placeholders::NumberedSqlite, &[]);
        assert_eq!(
            compiled.build(&[Arg::Flag(false)]).0,
            "SELECT a.id\nFROM assets a\nWHERE\n  a.deleted = 0\nORDER BY a.id"
        );
        assert_eq!(
            compiled.build(&[Arg::Flag(true)]).0,
            "SELECT a.id\nFROM assets a\nLEFT JOIN reviews r ON r.asset_id = a.id\nWHERE\n  a.deleted = 0\n  AND r.asset_id IS NOT NULL\nORDER BY a.id"
        );

        let info = parse_with_catalog(
            "SELECT a.id\nFROM assets a\nJOIN assets_fts ON assets_fts.rowid = a.id AND assets_fts MATCH ?1 -- :if @fts_match\nWHERE a.deleted = 0\nORDER BY\n  bm25(assets_fts) ASC, -- :if @fts_match\n  a.id ASC",
            &params(&["fts_match"]),
            Dialect::Sqlite,
        )
        .unwrap()
        .unwrap();
        let compiled = compile(&info, Placeholders::NumberedSqlite, &[1]);
        assert_eq!(
            compiled.build(&[Arg::Inactive]).0,
            "SELECT a.id\nFROM assets a\nWHERE a.deleted = 0\nORDER BY\n  a.id ASC"
        );
        assert_eq!(
            compiled.build(&[Arg::Active]).0,
            "SELECT a.id\nFROM assets a\nJOIN assets_fts ON assets_fts.rowid = a.id AND assets_fts MATCH $1\nWHERE a.deleted = 0\nORDER BY\n  bm25(assets_fts) ASC,\n  a.id ASC"
        );
    }
}

mod plans {
    use crate::dynfilter::test_support::{compile, params, parse_uncatalogued};
    use crate::dynfilter::{Connector, Dialect, PlanClause};
    use crate::dynfilter_runtime::{Arg, Bind, Placeholders};

    #[test]
    fn plans_empty_singleton_nested_and_all_inactive_shapes() {
        // No gated clause: the query is untouched and the plan is empty.
        let info = parse_uncatalogued(
            "SELECT id FROM t\nWHERE a = $1 -- :if @a\nORDER BY id",
            &params(&["a"]),
            Dialect::PostgreSql,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            info.annotated_sql,
            "SELECT id FROM t\nWHERE\n  a = $1 -- :if $1\nORDER BY id"
        );
        assert_eq!(
            info.plan,
            [PlanClause {
                header: 1,
                connector: Connector::And,
                items: vec![2],
            }]
        );
        let compiled = compile(&info, Placeholders::Numbered, &[1]);
        assert_eq!(
            compiled.build(&[Arg::Inactive]),
            ("SELECT id FROM t\nORDER BY id".to_string(), vec![])
        );
        assert_eq!(
            compiled.build(&[Arg::Active]),
            (
                "SELECT id FROM t\nWHERE\n  a = $1\nORDER BY id".to_string(),
                vec![Bind::Arg(0)]
            )
        );

        // A nested gate inside an all-annotated OR group with an ungated sibling clause.
        let info = parse_uncatalogued(
            "SELECT id FROM t\nWHERE\n  x > 0\n  AND ( -- :flag @g\n    a = $1 -- :if @a\n    OR EXISTS (SELECT 1 FROM u WHERE u.b = $2) -- :if @b\n  )\nORDER BY id",
            &params(&["a", "b"]),
            Dialect::PostgreSql,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            info.plan,
            [
                PlanClause {
                    header: 1,
                    connector: Connector::And,
                    items: vec![2, 3],
                },
                PlanClause {
                    header: 4,
                    connector: Connector::Or,
                    items: vec![5, 6, 7],
                },
            ]
        );
        let compiled = compile(&info, Placeholders::Numbered, &[1, 2]);
        assert_eq!(
            compiled
                .build(&[Arg::Inactive, Arg::Inactive, Arg::Flag(true)])
                .0,
            "SELECT id FROM t\nWHERE\n  x > 0\n  AND (\n\n    FALSE\n  )\nORDER BY id"
        );
        assert_eq!(
            compiled.build(&[Arg::Active, Arg::Active, Arg::Flag(false)]),
            (
                "SELECT id FROM t\nWHERE\n  x > 0\nORDER BY id".to_string(),
                vec![]
            )
        );
        assert_eq!(
            compiled.build(&[Arg::Inactive, Arg::Active, Arg::Flag(true)]),
            (
                "SELECT id FROM t\nWHERE\n  x > 0\n  AND (\n\n    FALSE\n    OR EXISTS (SELECT 1 FROM u WHERE u.b = $1)\n  )\nORDER BY id".to_string(),
                vec![Bind::Arg(1)]
            )
        );
    }

    #[test]
    fn collects_long_conjunction_chains() {
        let conjuncts = (0..300)
            .map(|index| format!("  AND c{index} = {index}"))
            .collect::<Vec<_>>()
            .join("\n");
        let sql = format!("SELECT id FROM t\nWHERE\n  a = $1 -- :if @a\n{conjuncts}");
        let info = parse_uncatalogued(&sql, &params(&["a"]), Dialect::PostgreSql)
            .unwrap()
            .unwrap();
        assert_eq!(info.plan[0].items.len(), 301);
        let compiled = compile(&info, Placeholders::Numbered, &[1]);
        let (off, binds) = compiled.build(&[Arg::Inactive]);
        assert!(off.starts_with("SELECT id FROM t\nWHERE\n  c0 = 0\n  AND c1 = 1\n"));
        assert!(binds.is_empty());
    }
}

mod matrix {
    use crate::dynfilter::Dialect;
    use crate::dynfilter::test_support::{compile, params, parse_catalogued};
    use crate::dynfilter_runtime::{Arg, Bind, Placeholders};
    use sqlparser::{dialect::PostgreSqlDialect, parser::Parser};

    const SQL: &str = "SELECT u.id FROM users u\nJOIN orders o ON o.user_id = u.id -- :flag @joined\nWHERE\n  u.id > 0\n  AND ( -- :flag @group\n    o.id > 0 -- :flag @joined\n    OR u.email = $1 -- :if @email\n  )\nORDER BY -- :switch @sort id_desc email_asc default=id_desc\n  u.id DESC, -- :case @id_desc\n  u.email ASC -- :case @email_asc\nLIMIT 10";

    /// Every combination of the optional bind, the JOIN flag, the enclosing group flag, and
    /// the two-way sort switch; argument order is `[email, joined, group, id_desc, email_asc]`.
    #[test]
    fn combines_join_group_bind_and_switch_gates() {
        let info = parse_catalogued(SQL, &params(&["email"]), Dialect::PostgreSql)
            .unwrap()
            .unwrap();
        let compiled = compile(&info, Placeholders::Numbered, &[1]);
        for state in 0..16_u8 {
            let (email, joined, group, id_desc) = (
                state & 1 != 0,
                state & 2 != 0,
                state & 4 != 0,
                state & 8 != 0,
            );
            let args = [
                if email { Arg::Active } else { Arg::Inactive },
                Arg::Flag(joined),
                Arg::Flag(group),
                Arg::Flag(id_desc),
                Arg::Flag(!id_desc),
            ];
            let (sql, binds) = compiled.build(&args);
            assert!(
                Parser::parse_sql(&PostgreSqlDialect {}, &sql).is_ok(),
                "{args:?}\n{sql}"
            );
            assert_eq!(sql.contains("JOIN orders o"), joined, "{args:?}\n{sql}");
            assert_eq!(sql.contains("o.id > 0"), joined && group, "{args:?}\n{sql}");
            assert_eq!(
                sql.contains("u.email = $1"),
                email && group,
                "{args:?}\n{sql}"
            );
            assert_eq!(
                binds,
                if email && group {
                    vec![Bind::Arg(0)]
                } else {
                    vec![]
                },
                "{args:?}\n{sql}"
            );
            assert_eq!(sql.contains("u.id DESC"), id_desc, "{args:?}\n{sql}");
            assert_eq!(sql.contains("u.email ASC"), !id_desc, "{args:?}\n{sql}");
            assert!(sql.ends_with("LIMIT 10"), "{args:?}\n{sql}");
        }
    }

    /// The same shape on MySQL with a slice operand: `?` binds follow the emitted order, and
    /// an empty slice renders `NULL`.
    #[test]
    fn orders_mysql_binds_for_scalars_and_slices_in_the_group() {
        let sql = "SELECT u.id FROM users u\nJOIN orders o ON o.user_id = u.id -- :flag @joined\nWHERE\n  u.id > 0\n  AND ( -- :flag @group\n    o.id > 0 -- :flag @joined\n    OR u.email = ? -- :if @email\n    OR u.id IN (/*SLICE:ids*/?) -- :if @ids\n  )\nORDER BY u.id DESC";
        let info = parse_catalogued(sql, &params(&["email", "ids"]), Dialect::MySql)
            .unwrap()
            .unwrap();
        let compiled = compile(&info, Placeholders::Question, &[1, 2]);
        for (ids, expected_sql, expected_binds) in [
            (
                Arg::Slice(None),
                "SELECT u.id FROM users u\nWHERE\n  u.id > 0\n  AND (\n\n    FALSE\n    OR u.email = ?\n  )\nORDER BY u.id DESC",
                vec![Bind::Arg(0)],
            ),
            (
                Arg::Slice(Some(0)),
                "SELECT u.id FROM users u\nWHERE\n  u.id > 0\n  AND (\n\n    FALSE\n    OR u.email = ?\n    OR u.id IN (NULL)\n  )\nORDER BY u.id DESC",
                vec![Bind::Arg(0)],
            ),
            (
                Arg::Slice(Some(2)),
                "SELECT u.id FROM users u\nWHERE\n  u.id > 0\n  AND (\n\n    FALSE\n    OR u.email = ?\n    OR u.id IN (?,?)\n  )\nORDER BY u.id DESC",
                vec![Bind::Arg(0), Bind::Elem(1, 0), Bind::Elem(1, 1)],
            ),
        ] {
            let (actual, binds) =
                compiled.build(&[Arg::Active, ids, Arg::Flag(false), Arg::Flag(true)]);
            assert_eq!(actual, expected_sql, "{ids:?}");
            assert_eq!(binds, expected_binds, "{ids:?}");
        }
        let (actual, binds) = compiled.build(&[
            Arg::Inactive,
            Arg::Slice(Some(1)),
            Arg::Flag(true),
            Arg::Flag(true),
        ]);
        assert_eq!(
            actual,
            "SELECT u.id FROM users u\nJOIN orders o ON o.user_id = u.id\nWHERE\n  u.id > 0\n  AND (\n\n    FALSE\n    OR o.id > 0\n    OR u.id IN (?)\n  )\nORDER BY u.id DESC"
        );
        assert_eq!(binds, [Bind::Elem(1, 0)]);
    }
}

mod or_groups {
    use crate::dynfilter::test_support::{compile, params, parse_uncatalogued};
    use crate::dynfilter::{Connector, Dialect, PlanClause};
    use crate::dynfilter_runtime::{Arg, Bind, Placeholders};

    #[test]
    fn gates_or_operands_in_each_dialect() {
        for (dialect, placeholders, a, b, out) in [
            (Dialect::PostgreSql, Placeholders::Numbered, "$1", "$2", "$"),
            (
                Dialect::Sqlite,
                Placeholders::NumberedSqlite,
                "?1",
                "?2",
                "$",
            ),
            (Dialect::MySql, Placeholders::Question, "?", "?", "?"),
        ] {
            let sql = format!(
                "SELECT id FROM t\nWHERE\n  account_id = 7\n  AND (\n    a LIKE {a} -- :if @a\n    OR b LIKE {b} -- :if @b\n  )\nORDER BY id"
            );
            let info = parse_uncatalogued(&sql, &params(&["a", "b"]), dialect)
                .unwrap()
                .unwrap();
            assert_eq!(
                info.plan,
                [PlanClause {
                    header: 4,
                    connector: Connector::Or,
                    items: vec![5, 6, 7]
                }],
                "{}",
                info.annotated_sql
            );
            let compiled = compile(&info, placeholders, &[1, 2]);
            let number = |n: usize| {
                if out == "?" {
                    "?".to_string()
                } else {
                    format!("${n}")
                }
            };

            let (none, binds) = compiled.build(&[Arg::Inactive, Arg::Inactive]);
            assert_eq!(
                none,
                "SELECT id FROM t\nWHERE\n  account_id = 7\n  AND (\n\n    FALSE\n  )\nORDER BY id"
            );
            assert!(binds.is_empty());

            let (second, binds) = compiled.build(&[Arg::Inactive, Arg::Active]);
            assert_eq!(
                second,
                format!(
                    "SELECT id FROM t\nWHERE\n  account_id = 7\n  AND (\n\n    FALSE\n    OR b LIKE {}\n  )\nORDER BY id",
                    number(1)
                )
            );
            assert_eq!(binds, [Bind::Arg(1)]);

            let (both, binds) = compiled.build(&[Arg::Active, Arg::Active]);
            assert_eq!(
                both,
                format!(
                    "SELECT id FROM t\nWHERE\n  account_id = 7\n  AND (\n\n    FALSE\n    OR a LIKE {}\n    OR b LIKE {}\n  )\nORDER BY id",
                    number(1),
                    number(2)
                )
            );
            assert_eq!(binds, [Bind::Arg(0), Bind::Arg(1)]);
        }
    }

    #[test]
    fn keeps_unannotated_operands_without_a_synthetic_false() {
        let info = parse_uncatalogued(
            "SELECT id FROM t\nWHERE (\n  a = $1 -- :if @a\n  OR b = 1\n  OR c = $2 -- :if @c\n)",
            &params(&["a", "c"]),
            Dialect::PostgreSql,
        )
        .unwrap()
        .unwrap();
        let compiled = compile(&info, Placeholders::Numbered, &[1, 2]);
        assert_eq!(
            compiled.build(&[Arg::Inactive, Arg::Inactive]).0,
            "SELECT id FROM t\nWHERE (\n\n  b = 1\n)"
        );
        assert_eq!(
            compiled.build(&[Arg::Inactive, Arg::Active]).0,
            "SELECT id FROM t\nWHERE (\n\n  b = 1\n  OR c = $1\n)"
        );
        assert_eq!(
            compiled.build(&[Arg::Active, Arg::Inactive]).0,
            "SELECT id FROM t\nWHERE (\n\n  a = $1\n  OR b = 1\n)"
        );
    }

    #[test]
    fn composes_group_conditions_with_gated_conjuncts_and_subqueries() {
        let info = parse_uncatalogued(
            "SELECT id FROM t\nWHERE\n  id > 0\n  AND ( -- :flag @by_name\n    a = $1 -- :if @a\n    OR b = $2 -- :if @b\n  )\n  AND EXISTS (\n    SELECT 1 FROM u\n    WHERE u.t = t.id\n      AND (\n        u.x = $3 -- :if @x\n        -- :if @y\n        OR u.y\n          = $4\n      )\n  )",
            &params(&["a", "b", "x", "y"]),
            Dialect::PostgreSql,
        )
        .unwrap()
        .unwrap();
        let compiled = compile(&info, Placeholders::Numbered, &[1, 2, 3, 4]);
        let inactive = Arg::Inactive;
        assert_eq!(
            compiled
                .build(&[inactive, inactive, inactive, inactive, Arg::Flag(false)])
                .0,
            "SELECT id FROM t\nWHERE\n  id > 0\n  AND EXISTS (\n    SELECT 1 FROM u\n    WHERE u.t = t.id\n      AND (\n\n        FALSE\n      )\n  )"
        );
        let (sql, binds) = compiled.build(&[
            Arg::Active,
            inactive,
            inactive,
            Arg::Active,
            Arg::Flag(true),
        ]);
        assert_eq!(
            sql,
            "SELECT id FROM t\nWHERE\n  id > 0\n  AND (\n\n    FALSE\n    OR a = $1\n  )\n  AND EXISTS (\n    SELECT 1 FROM u\n    WHERE u.t = t.id\n      AND (\n\n        FALSE\n        OR u.y\n          = $2\n      )\n  )"
        );
        assert_eq!(binds, [Bind::Arg(0), Bind::Arg(3)]);
    }

    #[test]
    fn locates_group_parens_around_parenthesized_operands() {
        let info = parse_uncatalogued(
            "SELECT id FROM t\nWHERE\n  x = 1\n  AND (\n    (a = $1 AND c = 2) -- :if @a\n    OR b = $2 -- :if @b\n    OR (d = 3 AND e = $3) -- :if @e\n  )",
            &params(&["a", "b", "e"]),
            Dialect::PostgreSql,
        )
        .unwrap()
        .unwrap();
        let compiled = compile(&info, Placeholders::Numbered, &[1, 2, 3]);
        assert_eq!(
            compiled
                .build(&[Arg::Inactive, Arg::Inactive, Arg::Inactive])
                .0,
            "SELECT id FROM t\nWHERE\n  x = 1\n  AND (\n\n    FALSE\n  )"
        );
        assert_eq!(
            compiled.build(&[Arg::Active, Arg::Inactive, Arg::Active]).0,
            "SELECT id FROM t\nWHERE\n  x = 1\n  AND (\n\n    FALSE\n    OR (a = $1 AND c = 2)\n    OR (d = 3 AND e = $2)\n  )"
        );

        let info = parse_uncatalogued(
            "SELECT id FROM t\nWHERE (\n  (a = $1) -- :if @a\n  OR b = 1\n)",
            &params(&["a"]),
            Dialect::PostgreSql,
        )
        .unwrap()
        .unwrap();
        let compiled = compile(&info, Placeholders::Numbered, &[1]);
        assert_eq!(
            compiled.build(&[Arg::Inactive]).0,
            "SELECT id FROM t\nWHERE (\n\n  b = 1\n)"
        );
        assert_eq!(
            compiled.build(&[Arg::Active]).0,
            "SELECT id FROM t\nWHERE (\n\n  (a = $1)\n  OR b = 1\n)"
        );
    }

    #[test]
    fn a_standalone_line_after_the_opening_paren_gates_the_first_operand() {
        let info = parse_uncatalogued(
            "SELECT id FROM t\nWHERE\n  x = 1\n  AND (\n    -- :if @a\n    a = $1\n    OR b = $2 -- :if @b\n  )",
            &params(&["a", "b"]),
            Dialect::PostgreSql,
        )
        .unwrap()
        .unwrap();
        let compiled = compile(&info, Placeholders::Numbered, &[1, 2]);
        assert_eq!(
            compiled.build(&[Arg::Inactive, Arg::Active]).0,
            "SELECT id FROM t\nWHERE\n  x = 1\n  AND (\n\n    FALSE\n    OR b = $1\n  )"
        );
        assert_eq!(
            compiled.build(&[Arg::Active, Arg::Inactive]).0,
            "SELECT id FROM t\nWHERE\n  x = 1\n  AND (\n\n    FALSE\n    OR a = $1\n  )"
        );
    }

    #[test]
    fn an_annotation_after_the_closing_paren_gates_the_conjunct() {
        let info = parse_uncatalogued(
            "SELECT id FROM t\nWHERE (\n  a = $1 -- :if @a\n  OR b = $2) -- :if @b",
            &params(&["a", "b"]),
            Dialect::PostgreSql,
        )
        .unwrap()
        .unwrap();
        let compiled = compile(&info, Placeholders::Numbered, &[1, 2]);
        assert_eq!(
            compiled.build(&[Arg::Active, Arg::Inactive]).0,
            "SELECT id FROM t"
        );
        assert_eq!(
            compiled.build(&[Arg::Inactive, Arg::Active]).0,
            "SELECT id FROM t\nWHERE\n  (\n\n  b = $1\n  )"
        );
    }

    #[test]
    fn rejects_bare_or_operands_switches_and_cases_in_groups() {
        for (sql, expected) in [
            (
                "SELECT id FROM t WHERE a = $1 -- :if @a\n OR b = 1",
                "does not attach",
            ),
            (
                "SELECT id FROM t\nWHERE (\n  a = $1 -- :case @a\n  OR b = 1\n)",
                "without a `-- :switch`",
            ),
            (
                "SELECT id FROM t\nWHERE ( -- :switch @s x y default=x\n  a = 1 -- :case @x\n  OR b = 1 -- :case @y\n)",
                "must follow a WHERE or ORDER BY keyword",
            ),
        ] {
            let error = parse_uncatalogued(sql, &params(&["a"]), Dialect::PostgreSql).unwrap_err();
            assert!(error.contains(expected), "{sql}\n{error}");
        }
    }
}

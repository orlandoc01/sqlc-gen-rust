// Tests for the runtime emitted into generated code; compiled only into the plugin, so
// consumer crates do not carry them.

use super::*;

/// Arguments are numbered in slot order, the way sqlc reports positional parameters.
fn order(args: &[Arg]) -> Vec<usize> {
    (1..=args.len()).collect()
}

/// The engine each placeholder form ships with.
pub(crate) fn dialect_of(placeholders: Placeholders) -> Dialect {
    match placeholders {
        Placeholders::Numbered => Dialect::Postgres,
        Placeholders::NumberedSqlite => Dialect::Sqlite,
        Placeholders::Question => Dialect::MySql,
    }
}

fn build(sql: &str, placeholders: Placeholders, args: &[Arg]) -> (String, Vec<Bind>) {
    compile_with_arg_order(sql, placeholders, dialect_of(placeholders), &order(args), &[]).build(args)
}

#[test]
fn scans_non_ascii_comments_for_markers_without_panicking() {
    let (sql, binds) = build(
        "SELECT id FROM t -- café\nWHERE id = $1 -- :if $1\nORDER BY id",
        Placeholders::Numbered,
        &[Arg::Inactive],
    );
    assert_eq!(sql, "SELECT id FROM t -- café\nORDER BY id");
    assert!(binds.is_empty());
}

fn build_plan(sql: &str, plan: &[Clause<'_>], args: &[Arg]) -> String {
    compile_with_arg_order(sql, Placeholders::Numbered, Dialect::Postgres, &order(args), plan)
        .build(args)
        .0
}

const WHERE_AND_ORDER: &[Clause<'_>] = &[
    Clause {
        header: 1,
        connector: Connector::And,
        items: &[2],
    },
    Clause {
        header: 3,
        connector: Connector::Comma,
        items: &[4, 5],
    },
];

#[test]
fn drops_inactive_conditions_and_remaps_gaps() {
    let (sql, binds) = build(
        "SELECT * FROM t\nWHERE a = $1\n  AND b = $2 -- :if $2\n  AND c = $3",
        Placeholders::Numbered,
        &[Arg::Active, Arg::Inactive, Arg::Active],
    );
    assert_eq!(sql, "SELECT * FROM t\nWHERE a = $1\n  AND c = $2");
    assert_eq!(binds, [Bind::Arg(0), Bind::Arg(2)]);
}

#[test]
fn keeps_multi_parameter_lines_only_when_all_are_active() {
    let sql = "SELECT * FROM t\nWHERE a = $1 -- :if $1 -- :if $2";
    assert_eq!(
        build(sql, Placeholders::Numbered, &[Arg::Active, Arg::Inactive]).0,
        "SELECT * FROM t"
    );
    assert_eq!(
        build(sql, Placeholders::Numbered, &[Arg::Active, Arg::Active]).0,
        "SELECT * FROM t\nWHERE a = $1"
    );
}

#[test]
fn numbered_placeholders_reuse_repeated_arguments() {
    let (sql, binds) = build(
        "SELECT * FROM t WHERE a = $1 OR b = $1",
        Placeholders::Numbered,
        &[Arg::Active],
    );
    assert_eq!(sql, "SELECT * FROM t WHERE a = $1 OR b = $1");
    assert_eq!(binds, [Bind::Arg(0)]);
}

#[test]
fn mysql_reemits_repeated_arguments() {
    let (sql, binds) = build(
        "SELECT * FROM t WHERE a = ? OR b = ?",
        Placeholders::Question,
        &[Arg::Active, Arg::Active],
    );
    assert_eq!(sql, "SELECT * FROM t WHERE a = ? OR b = ?");
    assert_eq!(binds, [Bind::Arg(0), Bind::Arg(1)]);
}

#[test]
fn mysql_expands_numbered_slice_markers() {
    let (sql, binds) = build(
        "SELECT * FROM t WHERE kind = ? AND id IN (/*SLICE:ids*/?2) -- :if $2",
        Placeholders::Question,
        &[Arg::Active, Arg::Slice(Some(2))],
    );
    assert_eq!(sql, "SELECT * FROM t WHERE kind = ? AND id IN (?,?)");
    assert_eq!(
        binds,
        [Bind::Arg(0), Bind::Elem(1, 0), Bind::Elem(1, 1)]
    );
}

#[test]
fn expands_slices_and_preserves_nil_empty_distinction() {
    let sql = "SELECT * FROM t\nWHERE name = ?1\n  AND id IN (/*SLICE:ids*/?2) -- :if $2";
    assert_eq!(
        build(
            sql,
            Placeholders::NumberedSqlite,
            &[Arg::Active, Arg::Slice(None)],
        )
        .0,
        "SELECT * FROM t\nWHERE name = $1"
    );
    assert_eq!(
        build(
            sql,
            Placeholders::NumberedSqlite,
            &[Arg::Active, Arg::Slice(Some(0))],
        ),
        (
            "SELECT * FROM t\nWHERE name = $1\n  AND id IN (NULL)".into(),
            vec![Bind::Arg(0)]
        )
    );
    assert_eq!(
        build(
            sql,
            Placeholders::NumberedSqlite,
            &[Arg::Active, Arg::Slice(Some(2))],
        ),
        (
            "SELECT * FROM t\nWHERE name = $1\n  AND id IN ($2,$3)".into(),
            vec![Bind::Arg(0), Bind::Elem(1, 0), Bind::Elem(1, 1)]
        )
    );
}

#[test]
fn repeated_numbered_slices_replay_expansion() {
    let (sql, binds) = build(
        "SELECT * FROM t WHERE id IN (/*SLICE:ids*/$1) OR parent_id IN (/*SLICE:ids*/$1)",
        Placeholders::Numbered,
        &[Arg::Slice(Some(2))],
    );
    assert_eq!(sql, "SELECT * FROM t WHERE id IN ($1,$2) OR parent_id IN ($1,$2)");
    assert_eq!(binds, [Bind::Elem(0, 0), Bind::Elem(0, 1)]);
}

#[test]
fn empty_repeated_slices_render_null_each_time() {
    assert_eq!(
        build(
            "SELECT * FROM t WHERE id IN (/*SLICE:ids*/$1) OR parent_id IN (/*SLICE:ids*/$1)",
            Placeholders::NumberedSqlite,
            &[Arg::Slice(Some(0))],
        )
        .0,
        "SELECT * FROM t WHERE id IN (NULL) OR parent_id IN (NULL)"
    );
}

#[test]
fn missing_slice_argument_renders_null() {
    assert_eq!(
        build(
            "SELECT * FROM t WHERE id IN (/*SLICE:ids*/?2) -- :if $1",
            Placeholders::NumberedSqlite,
            &[Arg::Active],
        ),
        ("SELECT * FROM t WHERE id IN (NULL)".into(), vec![])
    );
}

#[test]
fn drops_headers_and_connectors_of_emptied_clauses() {
    let sql = "SELECT * FROM t\nWHERE\n  a = $1 -- :if $1\nORDER BY\n  id ASC, -- :if $2\n  id DESC -- :if $3\nLIMIT 10";
    assert_eq!(
        build_plan(
            sql,
            WHERE_AND_ORDER,
            &[Arg::Inactive, Arg::Flag(false), Arg::Flag(false)]
        ),
        "SELECT * FROM t\nLIMIT 10"
    );
    assert_eq!(
        build_plan(
            sql,
            WHERE_AND_ORDER,
            &[Arg::Inactive, Arg::Flag(true), Arg::Flag(false)]
        ),
        "SELECT * FROM t\nORDER BY\n  id ASC\nLIMIT 10"
    );
    assert_eq!(
        build_plan(
            sql,
            WHERE_AND_ORDER,
            &[Arg::Active, Arg::Flag(false), Arg::Flag(true)]
        ),
        "SELECT * FROM t\nWHERE\n  a = $1\nORDER BY\n  id DESC\nLIMIT 10"
    );
}

#[test]
fn reconnects_or_operands_after_a_synthetic_false() {
    let sql = "SELECT * FROM t\nWHERE\n  x = 1\n  AND (\n\n    FALSE\n    OR a = $1 -- :if $1\n    OR b = $2 -- :if $2\n  )";
    let plan = [Clause {
        header: 4,
        connector: Connector::Or,
        items: &[5, 6, 7],
    }];
    assert_eq!(
        build_plan(sql, &plan, &[Arg::Inactive, Arg::Inactive]),
        "SELECT * FROM t\nWHERE\n  x = 1\n  AND (\n\n    FALSE\n  )"
    );
    assert_eq!(
        build_plan(sql, &plan, &[Arg::Inactive, Arg::Active]),
        "SELECT * FROM t\nWHERE\n  x = 1\n  AND (\n\n    FALSE\n    OR b = $1\n  )"
    );
    let unconditional = "SELECT * FROM t\nWHERE (\n\n  a = $1 -- :if $1\n  OR b = 1\n)";
    let plan = [Clause {
        header: 2,
        connector: Connector::Or,
        items: &[3, 4],
    }];
    assert_eq!(
        build_plan(unconditional, &plan, &[Arg::Inactive]),
        "SELECT * FROM t\nWHERE (\n\n  b = 1\n)"
    );
    assert_eq!(
        build_plan(unconditional, &plan, &[Arg::Active]),
        "SELECT * FROM t\nWHERE (\n\n  a = $1\n  OR b = 1\n)"
    );
}

#[test]
fn keeps_unconditional_items_and_reconnects_survivors() {
    let sql = "SELECT * FROM t\nWHERE\n  a = $1 -- :if $1\n  AND b = 1\n  AND c = $2 -- :if $2\nORDER BY\n  id ASC, -- :if $3\n  name DESC";
    let plan = [
        Clause {
            header: 1,
            connector: Connector::And,
            items: &[2, 3, 4],
        },
        Clause {
            header: 5,
            connector: Connector::Comma,
            items: &[6, 7],
        },
    ];
    assert_eq!(
        build_plan(sql, &plan, &[Arg::Inactive, Arg::Active, Arg::Flag(false)]),
        "SELECT * FROM t\nWHERE\n  b = 1\n  AND c = $1\nORDER BY\n  name DESC"
    );
    assert_eq!(
        build_plan(sql, &plan, &[Arg::Active, Arg::Inactive, Arg::Flag(true)]),
        "SELECT * FROM t\nWHERE\n  a = $1\n  AND b = 1\nORDER BY\n  id ASC,\n  name DESC"
    );
}

#[test]
fn strips_and_restores_commas_of_multi_line_order_by_terms() {
    let sql = "SELECT * FROM t\nORDER BY\n  CASE WHEN status = $1 THEN 0 -- :if $1\n       ELSE 1 END ASC, -- :if $1\n  created_at DESC, -- :if $2\n  id ASC";
    let plan = [Clause {
        header: 1,
        connector: Connector::Comma,
        items: &[2, 4, 5],
    }];
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
        assert_eq!(build_plan(sql, &plan, &args), expected);
    }
}

#[test]
fn drops_nested_headers_and_fixes_nested_connectors() {
    let sql = "SELECT * FROM users\nWHERE\n  email = $1 -- :if $1\n  AND EXISTS ( -- :if $3\n    SELECT 1 FROM orders -- :if $3\n    WHERE -- :if $3\n      paid = $2 -- :if $3 -- :if $2\n      AND shipped -- :if $3 -- :if $4\n  ) -- :if $3\nLIMIT 10";
    let plan = [
        Clause {
            header: 1,
            connector: Connector::And,
            items: &[2, 3],
        },
        Clause {
            header: 5,
            connector: Connector::And,
            items: &[6, 7],
        },
    ];
    assert_eq!(
        build_plan(
            sql,
            &plan,
            &[Arg::Inactive, Arg::Inactive, Arg::Flag(true), Arg::Flag(true)]
        ),
        "SELECT * FROM users\nWHERE\n  EXISTS (\n    SELECT 1 FROM orders\n    WHERE\n      shipped\n  )\nLIMIT 10"
    );
    assert_eq!(
        build_plan(
            sql,
            &plan,
            &[Arg::Active, Arg::Inactive, Arg::Flag(true), Arg::Flag(false)]
        ),
        "SELECT * FROM users\nWHERE\n  email = $1\n  AND EXISTS (\n    SELECT 1 FROM orders\n  )\nLIMIT 10"
    );
    assert_eq!(
        build_plan(
            sql,
            &plan,
            &[Arg::Active, Arg::Active, Arg::Flag(false), Arg::Flag(true)]
        ),
        "SELECT * FROM users\nWHERE\n  email = $1\nLIMIT 10"
    );
}

#[test]
fn preserves_blank_lines() {
    assert_eq!(
        build(
            "SELECT *\n\nFROM users\nWHERE id = $1 -- :if $1",
            Placeholders::Numbered,
            &[Arg::Active],
        )
        .0,
        "SELECT *\n\nFROM users\nWHERE id = $1"
    );
}

#[test]
fn ignores_markers_in_literals_and_comments() {
    let (sql, binds) = build(
        "SELECT '?1', '$2' /* $3 */ FROM t WHERE a = $1 -- ignore $2",
        Placeholders::Numbered,
        &[Arg::Active],
    );
    assert_eq!(sql, "SELECT '?1', '$2' /* $3 */ FROM t WHERE a = $1 -- ignore $2");
    assert_eq!(binds, [Bind::Arg(0)]);
}

#[test]
fn keeps_postgres_question_operators_as_text_with_an_argument_order() {
    let sql = "SELECT '{\"a\":1}'::jsonb ? 'a', '{}'::jsonb ?| array['a'] FROM users WHERE id = $1 AND '{}'::jsonb ?& array['b'] -- :if $2";
    let compiled = compile_with_arg_order(sql, Placeholders::Numbered, Dialect::Postgres, &[1, 2], &[]);
    let (actual, binds) = compiled.build(&[Arg::Active, Arg::Active]);
    assert_eq!(actual, sql[..sql.len() - " -- :if $2".len()]);
    assert_eq!(binds, [Bind::Arg(0)]);
    let (actual, binds) = compiled.build(&[Arg::Active, Arg::Inactive]);
    assert_eq!(actual, "");
    assert!(binds.is_empty());

    let (actual, binds) =
        compile_with_arg_order("SELECT a = ? AND b = ?", Placeholders::Question, Dialect::MySql, &[2, 1], &[])
            .build(&[Arg::Active, Arg::Active]);
    assert_eq!(actual, "SELECT a = ? AND b = ?");
    assert_eq!(binds, [Bind::Arg(1), Bind::Arg(0)]);
    let (actual, binds) = compile_with_arg_order(
        "SELECT a = ? AND b IN (/*SLICE:ids*/?)",
        Placeholders::NumberedSqlite,
        Dialect::Sqlite,
        &[2, 1],
        &[],
    )
    .build(&[Arg::Slice(Some(1)), Arg::Active]);
    assert_eq!(actual, "SELECT a = $1 AND b IN ($2)");
    assert_eq!(binds, [Bind::Arg(1), Bind::Elem(0, 0)]);
}

#[test]
fn keeps_postgres_question_operators_as_text() {
    let (sql, binds) = build(
        "SELECT * FROM t WHERE json ? 'key' AND data?1 = 'x' AND a = $1",
        Placeholders::Numbered,
        &[Arg::Active],
    );
    assert_eq!(sql, "SELECT * FROM t WHERE json ? 'key' AND data?1 = 'x' AND a = $1");
    assert_eq!(binds, [Bind::Arg(0)]);
}

#[test]
fn handles_postgres_dollar_quotes_escape_strings_and_nested_comments() {
    let (sql, binds) = build(
        "SELECT E'it\\'s $2', $tag$ -- :if $3 $tag$ /* outer /* inner */ $4 */ FROM t WHERE a = $1",
        Placeholders::Numbered,
        &[Arg::Active],
    );
    assert_eq!(sql, "SELECT E'it\\'s $2', $tag$ -- :if $3 $tag$ /* outer /* inner */ $4 */ FROM t WHERE a = $1");
    assert_eq!(binds, [Bind::Arg(0)]);
}

#[test]
fn keeps_postgres_array_subscripts_as_placeholders() {
    let (sql, binds) = build(
        "SELECT tags[$1] FROM t WHERE a = $2",
        Placeholders::Numbered,
        &[Arg::Active, Arg::Active],
    );
    assert_eq!(sql, "SELECT tags[$1] FROM t WHERE a = $2");
    assert_eq!(binds, [Bind::Arg(0), Bind::Arg(1)]);
}

/// PostgreSQL brackets are expression syntax, never quoted identifiers: constructors,
/// arithmetic subscripts, padded subscripts, and slices all bind their parameters, and they
/// renumber after an earlier conditional bind disappears.
#[test]
fn scans_postgres_bracket_expressions_for_placeholders() {
    let sql = "SELECT vals[$2::int + 1], vals[ $3 ], vals[$2:$3] FROM t\nWHERE a = $1 -- :if $1\n  AND id = ANY(ARRAY[$2::bigint, $3::bigint])";
    let (actual, binds) = build(
        sql,
        Placeholders::Numbered,
        &[Arg::Active, Arg::Active, Arg::Active],
    );
    assert_eq!(
        actual,
        "SELECT vals[$1::int + 1], vals[ $2 ], vals[$1:$2] FROM t\nWHERE a = $3\n  AND id = ANY(ARRAY[$1::bigint, $2::bigint])"
    );
    assert_eq!(binds, [Bind::Arg(1), Bind::Arg(2), Bind::Arg(0)]);
    let (actual, binds) = build(
        sql,
        Placeholders::Numbered,
        &[Arg::Inactive, Arg::Active, Arg::Active],
    );
    assert_eq!(
        actual,
        "SELECT vals[$1::int + 1], vals[ $2 ], vals[$1:$2] FROM t\n  AND id = ANY(ARRAY[$1::bigint, $2::bigint])"
    );
    assert_eq!(binds, [Bind::Arg(1), Bind::Arg(2)]);
}

/// `$` continues an unquoted PostgreSQL identifier, so a digit suffix is never a parameter,
/// whether or not the number is a real argument index.
#[test]
fn keeps_dollar_signs_inside_identifiers() {
    let sql = "SELECT price$9, a$tag$b, x$1, \"q$1\" FROM t WHERE y = $1 AND z = $tag$ $2 $tag$";
    let (actual, binds) = build(sql, Placeholders::Numbered, &[Arg::Active]);
    assert_eq!(actual, sql);
    assert_eq!(binds, [Bind::Arg(0)]);

    let (actual, binds) = build(
        "SELECT price$9 FROM t\nWHERE x$1 = 1\n  AND y = $1 -- :if $1",
        Placeholders::Numbered,
        &[Arg::Inactive],
    );
    assert_eq!(actual, "SELECT price$9 FROM t\nWHERE x$1 = 1");
    assert!(binds.is_empty());
}

/// MySQL `#` comments hold no binds, quotes, or markers, and `--` only comments with a
/// following space.
#[test]
fn skips_mysql_hash_comments() {
    let sql = "SELECT id FROM t # it's ? a -- :if $1 'comment\nWHERE a = ? # literal ? here\n  AND b = ? -- :if $2";
    let (actual, binds) = build(sql, Placeholders::Question, &[Arg::Active, Arg::Active]);
    assert_eq!(actual, sql[..sql.len() - " -- :if $2".len()]);
    assert_eq!(binds, [Bind::Arg(0), Bind::Arg(1)]);
    let (actual, binds) = build(sql, Placeholders::Question, &[Arg::Active, Arg::Inactive]);
    assert_eq!(
        actual,
        "SELECT id FROM t # it's ? a -- :if $1 'comment\nWHERE a = ? # literal ? here"
    );
    assert_eq!(binds, [Bind::Arg(0)]);

    let (actual, binds) = build(
        "SELECT id FROM t WHERE a = 1 --? AND b = ?",
        Placeholders::Question,
        &[Arg::Active],
    );
    assert_eq!(actual, "SELECT id FROM t WHERE a = 1 --? AND b = ?");
    assert_eq!(binds, [Bind::Arg(0)]);
}

#[test]
fn hash_comments_inside_a_gated_block_stay_inert() {
    let sql = "SELECT id FROM t\nWHERE\n  a = ? -- :if $1\n  AND EXISTS ( -- :if $3\n    SELECT 1 FROM o # ? -- :if $1\n    WHERE o.t = t.id AND o.b = ? -- :if $3 -- :if $2\n  ) -- :if $3";
    let plan = [Clause {
        header: 1,
        connector: Connector::And,
        items: &[2, 3],
    }];
    let states = [
        (
            [Arg::Inactive, Arg::Active, Arg::Flag(true)],
            "SELECT id FROM t\nWHERE\n  EXISTS (\n    SELECT 1 FROM o # ? -- :if $1\n    WHERE o.t = t.id AND o.b = ?\n  )",
            vec![Bind::Arg(1)],
        ),
        (
            [Arg::Active, Arg::Inactive, Arg::Flag(true)],
            "SELECT id FROM t\nWHERE\n  a = ?\n  AND EXISTS (\n    SELECT 1 FROM o # ? -- :if $1\n  )",
            vec![Bind::Arg(0)],
        ),
    ];
    for (args, expected, binds) in states {
        let (actual, actual_binds) =
            compile_with_arg_order(sql, Placeholders::Question, Dialect::MySql, &[1, 2], &plan).build(&args);
        assert_eq!(actual, expected);
        assert_eq!(actual_binds, binds);
    }
}

/// Markers only ever follow SQL, so one behind ordinary comment prose is text, not a gate.
#[test]
fn markers_inside_ordinary_line_comments_do_not_gate() {
    let sql = "SELECT id -- Documentation example: -- :if $1\nFROM t\nWHERE a = $1 -- :if $1";
    let (actual, binds) = build(sql, Placeholders::Numbered, &[Arg::Inactive]);
    assert_eq!(actual, "SELECT id -- Documentation example: -- :if $1\nFROM t");
    assert!(binds.is_empty());
    let (actual, binds) = build(
        "SELECT id /* -- :if $1 */ FROM t WHERE a = $1 /* -- :if $9 */ -- :if $1",
        Placeholders::Numbered,
        &[Arg::Active],
    );
    assert_eq!(actual, "SELECT id /* -- :if $1 */ FROM t WHERE a = $1 /* -- :if $9 */");
    assert_eq!(binds, [Bind::Arg(0)]);
}

#[test]
fn keeps_sqlite_bracket_identifiers_opaque() {
    let (sql, binds) = build(
        "SELECT [a?1b], [x$2y] FROM t WHERE a = ?1",
        Placeholders::NumberedSqlite,
        &[Arg::Active],
    );
    assert_eq!(sql, "SELECT [a?1b], [x$2y] FROM t WHERE a = $1");
    assert_eq!(binds, [Bind::Arg(0)]);
}

#[test]
fn respects_mysql_backslash_escaped_strings() {
    let sql = "SELECT a FROM t\nWHERE cond = 1\n  AND name = 'O\\'Brien' AND status = ? -- :if $1";
    assert_eq!(
        build(sql, Placeholders::Question, &[Arg::Active]).0,
        "SELECT a FROM t\nWHERE cond = 1\n  AND name = 'O\\'Brien' AND status = ?"
    );
    assert_eq!(
        build(sql, Placeholders::Question, &[Arg::Inactive]).0,
        "SELECT a FROM t\nWHERE cond = 1"
    );
}

#[test]
fn ignores_annotations_on_multiline_literal_continuations() {
    let (sql, binds) = build(
        "SELECT * FROM t\nWHERE note = 'hello\n-- :if $1\nworld' AND active = $2",
        Placeholders::Numbered,
        &[Arg::Inactive, Arg::Active],
    );
    assert_eq!(sql, "SELECT * FROM t\nWHERE note = 'hello\n-- :if $1\nworld' AND active = $1");
    assert_eq!(binds, [Bind::Arg(1)]);
}

#[test]
fn nilable_turns_only_empty_slices_into_none() {
    assert_eq!(nilable::<i64>(&[]), None);
    assert_eq!(nilable(&[1_i64]), Some(&[1][..]));
    assert_eq!(Arg::from_option(&Some(1)), Arg::Active);
    assert_eq!(Arg::from_option::<i64>(&None), Arg::Inactive);
}

#[test]
fn lexes_by_dialect_rather_than_placeholder_form() {
    // `#` opens a MySQL line comment, hiding the marker; PostgreSQL reads it as an operator.
    let sql = "SELECT a # b FROM t WHERE c = ? -- :if $1";
    let mysql = compile_with_arg_order(sql, Placeholders::Question, Dialect::MySql, &[1], &[]);
    assert_eq!(mysql.build(&[Arg::Inactive]).0, sql);
    let postgres =
        compile_with_arg_order(sql, Placeholders::Question, Dialect::Postgres, &[1], &[]);
    assert_eq!(postgres.build(&[Arg::Inactive]).0, "");
    let (active, binds) = postgres.build(&[Arg::Active]);
    assert_eq!(active, "SELECT a # b FROM t WHERE c = ?");
    assert_eq!(binds, [Bind::Arg(0)]);
}

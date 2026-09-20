//! The structures dynamic filters can gate, collected from the AST and located in the source:
//! `WHERE` conjuncts, `OR` operands, `ORDER BY` terms, and joins.

use std::ops::ControlFlow;

use sqlparser::{
    ast::{
        BinaryOperator, Expr, Join, JoinConstraint, JoinOperator, OrderByKind, Select, SelectItem,
        SetExpr, Spanned, Statement, Visitor,
    },
    keywords::Keyword,
    tokenizer::{Span, Token},
};

use super::{
    Connector, resolve,
    source::{Range, Source, TokenAt, keyword},
};

pub(super) struct RawClause {
    pub(super) range: Range,
    pub(super) keyword: Range,
    pub(super) content: Range,
    pub(super) depth: usize,
}

pub(super) struct Candidate {
    pub(super) range: Range,
    pub(super) surface: Range,
    /// Identifies the clause whose items this structure belongs to, so an inline annotation
    /// can cover its siblings. It is assigned before `clauses` is sorted and joins receive
    /// synthetic numbers past the clause count: an attachment-group identity, never an index
    /// into `clauses`.
    pub(super) group: usize,
    /// Index into the query's annotations once one attaches to this structure.
    pub(super) annotation: Option<usize>,
    /// Bind slots of the gates this structure carries.
    pub(super) conditions: Vec<usize>,
    pub(super) join: Option<AstJoin>,
}

pub(super) struct Clause {
    pub(super) kind: ClauseKind,
    pub(super) range: Range,
    pub(super) keyword: Range,
    pub(super) items: Vec<usize>,
    /// Index of the `-- :switch` annotation attached to this clause.
    pub(super) switch: Option<usize>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ClauseKind {
    Where,
    OrderBy,
    /// A parenthesized disjunction that is a complete `WHERE` conjunct.
    Or,
}

pub(super) struct AstJoin {
    pub(super) relation: Span,
    pub(super) on: Option<Span>,
    pub(super) unsupported: Option<Unsupported>,
    /// The owning projection contains a bare `*`.
    pub(super) wildcard: bool,
}

pub(super) enum Unsupported {
    Operator(&'static str),
    Relation(&'static str),
}

#[derive(Default)]
pub(super) struct AstStructures {
    pub(super) selections: Vec<Vec<Span>>,
    pub(super) or_groups: Vec<Vec<Span>>,
    pub(super) orders: Vec<Vec<Span>>,
    pub(super) joins: Vec<AstJoin>,
}

impl AstStructures {
    pub(super) fn record_selection(&mut self, selection: &Expr) {
        let conjuncts = operands(selection, &BinaryOperator::And);
        self.selections
            .push(conjuncts.iter().map(|expr| expr.span()).collect());
        self.or_groups.extend(conjuncts.iter().filter_map(|expr| {
            match expr {
                Expr::Nested(inner)
                    if matches!(
                        &**inner,
                        Expr::BinaryOp {
                            op: BinaryOperator::Or,
                            ..
                        }
                    ) =>
                {
                    Some(
                        operands(inner, &BinaryOperator::Or)
                            .iter()
                            .map(|expr| expr.span())
                            .collect(),
                    )
                }
                _ => None,
            }
        }));
    }
}

impl Visitor for AstStructures {
    type Break = ();

    fn pre_visit_query(&mut self, query: &sqlparser::ast::Query) -> ControlFlow<Self::Break> {
        for select in selects(&query.body) {
            if let Some(selection) = &select.selection {
                self.record_selection(selection);
            }
            let wildcard = select
                .projection
                .iter()
                .any(|item| matches!(item, SelectItem::Wildcard(_)));
            self.joins.extend(
                select
                    .from
                    .iter()
                    .flat_map(|from| &from.joins)
                    .map(|join| ast_join(join, wildcard)),
            );
        }
        if let Some(order_by) = &query.order_by
            && let OrderByKind::Expressions(expressions) = &order_by.kind
        {
            self.orders
                .push(expressions.iter().map(Spanned::span).collect());
        }
        ControlFlow::Continue(())
    }

    fn pre_visit_statement(&mut self, statement: &Statement) -> ControlFlow<Self::Break> {
        let selection = match statement {
            Statement::Update(update) => update.selection.as_ref(),
            Statement::Delete(delete) => delete.selection.as_ref(),
            _ => None,
        };
        if let Some(selection) = selection {
            self.record_selection(selection);
        }
        ControlFlow::Continue(())
    }
}

/// The `SELECT`s a query body owns directly: one, or the sides of a set operation. A
/// parenthesized side is its own `Query` node and is visited separately.
pub(super) fn selects(body: &SetExpr) -> Vec<&Select> {
    let mut output = Vec::new();
    collect_selects(body, &mut output);
    output
}

fn collect_selects<'a>(body: &'a SetExpr, output: &mut Vec<&'a Select>) {
    match body {
        SetExpr::Select(select) => output.push(select),
        SetExpr::SetOperation { left, right, .. } => {
            collect_selects(left, output);
            collect_selects(right, output);
        }
        _ => {}
    }
}

/// The operands of a chain of `operator`, in source order; each is visited once.
pub(super) fn operands<'a>(expr: &'a Expr, operator: &BinaryOperator) -> Vec<&'a Expr> {
    let mut output = Vec::new();
    collect_operands(expr, operator, &mut output);
    output
}

fn collect_operands<'a>(expr: &'a Expr, operator: &BinaryOperator, output: &mut Vec<&'a Expr>) {
    match expr {
        Expr::BinaryOp { left, op, right } if op == operator => {
            collect_operands(left, operator, output);
            collect_operands(right, operator, output);
        }
        _ => output.push(expr),
    }
}

pub(super) fn ast_join(join: &Join, wildcard: bool) -> AstJoin {
    let (constraint, operator) = match &join.join_operator {
        JoinOperator::Join(constraint)
        | JoinOperator::Inner(constraint)
        | JoinOperator::Left(constraint)
        | JoinOperator::LeftOuter(constraint) => (Some(constraint), None),
        JoinOperator::Right(constraint) | JoinOperator::RightOuter(constraint) => {
            (Some(constraint), Some("a RIGHT JOIN"))
        }
        JoinOperator::FullOuter(constraint) => (Some(constraint), Some("a FULL JOIN")),
        JoinOperator::CrossJoin(constraint) => (Some(constraint), Some("a CROSS JOIN")),
        _ => (None, Some("this join type")),
    };
    let operator = operator.or(match constraint {
        Some(JoinConstraint::On(_)) => None,
        Some(JoinConstraint::Using(_)) => Some("a USING join"),
        Some(JoinConstraint::Natural) => Some("a NATURAL join"),
        Some(JoinConstraint::None) | None => Some("a join without ON"),
    });
    let unsupported = operator
        .map(Unsupported::Operator)
        .or_else(|| resolve::unsupported_factor(&join.relation).map(Unsupported::Relation));
    AstJoin {
        relation: join.relation.span(),
        on: match constraint {
            Some(JoinConstraint::On(expr)) => Some(expr.span()),
            _ => None,
        },
        unsupported,
        wildcard,
    }
}

pub(super) fn structures_from_ast(
    source: &Source<'_>,
    structures: AstStructures,
) -> Result<(Vec<Clause>, Vec<Candidate>), String> {
    let selections = structures
        .selections
        .iter()
        .map(|spans| keyword_clause(source, spans, ClauseKind::Where))
        .collect::<Result<Vec<_>, _>>()?;
    let orders = structures
        .orders
        .iter()
        .map(|spans| keyword_clause(source, spans, ClauseKind::OrderBy))
        .collect::<Result<Vec<_>, _>>()?;
    let groups = structures
        .or_groups
        .iter()
        .map(|spans| or_clause(source, spans))
        .collect::<Result<Vec<_>, _>>()?;
    let definitions = selections
        .into_iter()
        .map(|definition| (ClauseKind::Where, definition))
        .chain(
            orders
                .into_iter()
                .map(|definition| (ClauseKind::OrderBy, definition)),
        )
        .chain(
            groups
                .into_iter()
                .map(|definition| (ClauseKind::Or, definition)),
        )
        .collect::<Vec<_>>();
    let mut candidates = Vec::new();
    let mut clauses = definitions
        .into_iter()
        .enumerate()
        .map(|(group, (kind, (range, keyword, items)))| Clause {
            kind,
            range,
            keyword,
            items: items
                .into_iter()
                .map(|range| {
                    let surface = source.surface(range);
                    let index = candidates.len();
                    candidates.push(Candidate {
                        range,
                        surface,
                        group,
                        annotation: None,
                        conditions: Vec::new(),
                        join: None,
                    });
                    index
                })
                .collect(),
            switch: None,
        })
        .collect::<Vec<_>>();
    let first_join_group = clauses.len();
    for (offset, join) in structures.joins.into_iter().enumerate() {
        candidates.push(join_candidate(source, join, first_join_group + offset)?);
    }
    clauses.sort_unstable_by_key(|clause| (clause.range.start, usize::MAX - clause.range.end));
    Ok((clauses, candidates))
}

/// `Join::span()` covers the relation and the `ON` expression but not the `LEFT JOIN` keywords
/// or a derived table's parentheses, so the range extends backward over them at the join's
/// depth.
pub(super) fn join_candidate(
    source: &Source<'_>,
    join: AstJoin,
    group: usize,
) -> Result<Candidate, String> {
    let tokens = &source.tokens;
    let missing = || "dynamic filters could not locate a JOIN".to_string();
    let relation = source.range(join.relation).ok_or_else(missing)?;
    let on = join
        .on
        .map(|span| source.range(span).ok_or_else(missing))
        .transpose()?;
    let mut first = tokens
        .iter()
        .position(|token| token.range.start >= relation.start)
        .ok_or_else(missing)?;
    while let Some(previous) = first.checked_sub(1)
        && matches!(tokens[previous].token, Token::LParen)
        && tokens[previous].depth + 1 == tokens[first].depth
    {
        first = previous;
    }
    let depth = tokens[first].depth;
    let start = tokens[..first]
        .iter()
        .rev()
        .take_while(|token| {
            token.depth == depth
                && matches!(
                    keyword(&token.token),
                    Keyword::JOIN
                        | Keyword::INNER
                        | Keyword::LEFT
                        | Keyword::RIGHT
                        | Keyword::FULL
                        | Keyword::OUTER
                        | Keyword::CROSS
                        | Keyword::NATURAL
                        | Keyword::LATERAL
                )
        })
        .last()
        .map_or(tokens[first].range.start, |token| token.range.start);
    // Expression spans end at the last operand: `IN (...)` and `(a AND (b))` exclude their
    // closing parens, which sit at the join's depth or deeper.
    let end = on.map_or(relation.end, |on| on.end);
    let end = tokens
        .iter()
        .skip_while(|token| token.range.start < end)
        .take_while(|token| token.depth >= depth && matches!(token.token, Token::RParen))
        .last()
        .map_or(end, |token| token.range.end);
    let range = Range { start, end };
    Ok(Candidate {
        range,
        surface: source.surface(range),
        group,
        annotation: None,
        conditions: Vec::new(),
        join: Some(join),
    })
}

/// The group's range runs from the first operand to the closing parenthesis, so the `(` line
/// stays with the enclosing conjunct and `)` is rendered by the mid-line remainder path.
pub(super) fn or_clause(
    source: &Source<'_>,
    spans: &[Span],
) -> Result<(Range, Range, Vec<Range>), String> {
    let tokens = &source.tokens;
    let missing = || "dynamic filters could not locate an OR group".to_string();
    let items = item_ranges(source, spans, ClauseKind::Or)?;
    let (Some(&first), Some(&second), Some(&last)) = (items.first(), items.get(1), items.last())
    else {
        return Err(missing());
    };
    // `Expr::Nested` spans exclude their parens, so a parenthesized operand starts inside its
    // own `(`; the `OR` between the first two operands sits at the group's depth, and the
    // group's parens one level up.
    let depth = tokens
        .iter()
        .find(|token| {
            first.end <= token.range.start
                && token.range.end <= second.start
                && keyword(&token.token) == Keyword::OR
        })
        .ok_or_else(missing)?
        .depth;
    let paren_depth = depth.checked_sub(1).ok_or_else(missing)?;
    let open = tokens
        .iter()
        .rev()
        .find(|token| {
            token.range.end <= first.start
                && token.depth == paren_depth
                && matches!(token.token, Token::LParen)
        })
        .ok_or_else(missing)?;
    let close = tokens
        .iter()
        .find(|token| {
            last.end <= token.range.start
                && token.depth == paren_depth
                && matches!(token.token, Token::RParen)
        })
        .ok_or_else(missing)?;
    // Start at the first token after `(` so a nested first operand keeps its own `(` and a
    // comment gating the conjunct stays outside the first operand.
    let content_start = tokens
        .iter()
        .find(|token| open.range.end <= token.range.start)
        .map_or(open.range.end, |token| token.range.start);
    let content = source.trim(content_start, close.range.start);
    let raw = RawClause {
        range: Range {
            start: content.start,
            end: close.range.start,
        },
        keyword: open.range,
        content,
        depth,
    };
    clause(source, &raw, items, ClauseKind::Or)
}

/// A `WHERE` or `ORDER BY` clause located from its parsed items: the keyword is the last one
/// before the first item, and the clause runs from there to the first clause keyword or
/// depth change after the last item. Words inside the items never end the clause, so a
/// column named `returning` or `fetch` in a dialect that allows it is content, not a
/// terminator.
pub(super) fn keyword_clause(
    source: &Source<'_>,
    spans: &[Span],
    kind: ClauseKind,
) -> Result<(Range, Range, Vec<Range>), String> {
    let tokens = &source.tokens;
    let items = item_ranges(source, spans, kind)?;
    let missing = || {
        format!(
            "dynamic filters could not locate the {} clause of the {} on line {}",
            kind.name(),
            kind.item_name(),
            source.line_of(items[0].start) + 1
        )
    };
    let index = tokens
        .iter()
        .rposition(|token| token.range.end <= items[0].start && kind.opens_clause(&token.token))
        .ok_or_else(missing)?;
    let opener = &tokens[index];
    let content_start = if kind == ClauseKind::OrderBy {
        let by = tokens
            .get(index + 1)
            .filter(|by| keyword(&by.token) == Keyword::BY && by.depth == opener.depth);
        by.ok_or_else(missing)?.range.end
    } else {
        opener.range.end
    };
    let last = items.last().expect("item_ranges rejects an empty clause");
    let end = clause_end(tokens, last.end, opener.depth, source.sql.len());
    let raw = RawClause {
        range: Range {
            start: opener.range.start,
            end,
        },
        keyword: Range {
            start: opener.range.start,
            end: content_start,
        },
        content: source.trim(content_start, end),
        depth: opener.depth,
    };
    clause(source, &raw, items, kind)
}

fn item_ranges(
    source: &Source<'_>,
    spans: &[Span],
    kind: ClauseKind,
) -> Result<Vec<Range>, String> {
    spans
        .iter()
        .map(|span| source.range(*span))
        .collect::<Option<Vec<_>>>()
        .filter(|items| !items.is_empty())
        .ok_or_else(|| format!("dynamic filters could not locate a {}", kind.item_name()))
}

pub(super) fn clause(
    source: &Source<'_>,
    clause: &RawClause,
    items: Vec<Range>,
    kind: ClauseKind,
) -> Result<(Range, Range, Vec<Range>), String> {
    let tokens = &source.tokens;
    let separators = items
        .windows(2)
        .map(|window| {
            tokens
                .iter()
                .filter(|token| {
                    token.depth == clause.depth
                        && kind.is_separator(&token.token)
                        && window[0].end <= token.range.start
                        && token.range.end <= window[1].start
                })
                .next_back()
                .map(|token| token.range)
                .ok_or_else(|| {
                    format!(
                        "dynamic filters could not locate a {} separator",
                        kind.name()
                    )
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let first_start = tokens
        .iter()
        .find(|token| {
            token.depth == clause.depth
                && keyword(&token.token) == Keyword::AND
                && clause.content.start <= token.range.start
                && token.range.end <= items[0].start
        })
        .map_or(clause.content.start, |token| {
            source.trim(token.range.end, items[0].start).start
        });
    let starts =
        std::iter::once(first_start).chain(separators.iter().map(|separator| separator.end));
    let ends = separators
        .iter()
        .map(|separator| separator.start)
        .chain(std::iter::once(clause.content.end));
    Ok((
        clause.range,
        clause.keyword,
        starts
            .zip(ends)
            .map(|(start, end)| source.trim(start, end))
            .collect(),
    ))
}

impl ClauseKind {
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Where => "WHERE",
            Self::OrderBy => "ORDER BY",
            Self::Or => "OR group",
        }
    }

    pub(super) fn describe(self) -> &'static str {
        match self {
            Self::Where => "a WHERE clause",
            Self::OrderBy => "an ORDER BY clause",
            Self::Or => "an OR group",
        }
    }

    pub(super) fn connector(self) -> Connector {
        match self {
            Self::Where => Connector::And,
            Self::OrderBy => Connector::Comma,
            Self::Or => Connector::Or,
        }
    }

    pub(super) fn item_name(self) -> &'static str {
        match self {
            Self::Where => "WHERE conjunct",
            Self::OrderBy => "ORDER BY term",
            Self::Or => "OR operand",
        }
    }

    pub(super) fn is_separator(self, token: &Token) -> bool {
        match self {
            Self::Where => keyword(token) == Keyword::AND,
            Self::OrderBy => matches!(token, Token::Comma),
            Self::Or => keyword(token) == Keyword::OR,
        }
    }

    fn opens_clause(self, token: &Token) -> bool {
        match self {
            Self::Where => keyword(token) == Keyword::WHERE,
            Self::OrderBy => keyword(token) == Keyword::ORDER,
            Self::Or => false,
        }
    }
}

/// The first position at or after `from` where a clause at `depth` ends.
pub(super) fn clause_end(tokens: &[TokenAt], from: usize, depth: usize, default: usize) -> usize {
    tokens
        .iter()
        .skip_while(|token| token.range.start < from)
        .find(|token| {
            token.depth < depth
                || (token.depth == depth
                    && (matches!(token.token, Token::SemiColon) || clause_keyword(&token.token)))
        })
        .map_or(default, |token| token.range.start)
}

pub(super) fn clause_keyword(token: &Token) -> bool {
    matches!(
        keyword(token),
        Keyword::GROUP
            | Keyword::HAVING
            | Keyword::ORDER
            | Keyword::LIMIT
            | Keyword::OFFSET
            | Keyword::FETCH
            | Keyword::RETURNING
            | Keyword::WINDOW
            | Keyword::UNION
            | Keyword::EXCEPT
            | Keyword::INTERSECT
            | Keyword::FOR
    )
}

/// Bind slots of every gate in effect at `position`: the structures containing it, outermost
/// first.
pub(super) fn gates_at(
    candidates: &[Candidate],
    position: usize,
) -> impl Iterator<Item = usize> + '_ {
    candidates
        .iter()
        .filter(move |candidate| candidate.range.contains_position(position))
        .flat_map(|candidate| candidate.conditions.iter().copied())
}

pub(super) fn covered(candidates: &[Candidate], position: usize, required: &[usize]) -> bool {
    let active = gates_at(candidates, position).collect::<Vec<_>>();
    required.iter().all(|gate| active.contains(gate))
}

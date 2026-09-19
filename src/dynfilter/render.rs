//! Renders the canonical annotated SQL and the line plan the runtime compiles.

use std::fmt::Write as _;

use super::{
    PlanClause,
    source::{Range, Source, first_non_whitespace},
    structures::{Candidate, Clause, ClauseKind, gates_at},
};

pub(super) fn blank_gated_comments(
    sql: &str,
    candidates: &[Candidate],
    comments: &[Range],
) -> String {
    let mut bytes = sql.as_bytes().to_vec();
    for comment in comments.iter().filter(|comment| {
        candidates
            .iter()
            .any(|candidate| candidate.annotation.is_some() && candidate.range.contains(**comment))
    }) {
        for byte in &mut bytes[comment.start..comment.end] {
            if *byte != b'\n' {
                *byte = b' ';
            }
        }
    }
    String::from_utf8(bytes).expect("blanking replaces whole bytes with ASCII spaces")
}

pub(super) struct Renderer<'a> {
    pub(super) source: &'a Source<'a>,
    /// The query text with gated block comments blanked out; positions match `source`.
    pub(super) text: &'a str,
    /// Line comments to drop from the rendered text, in source order.
    pub(super) comments: &'a [Range],
    pub(super) clauses: &'a [Clause],
    pub(super) candidates: &'a [Candidate],
    /// String literals and quoted identifiers spanning several lines: their bytes are data
    /// and are copied through untouched.
    pub(super) literals: &'a [Range],
}

pub(super) struct Line {
    pub(super) role: Role,
    pub(super) text: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Role {
    Plain,
    Header(usize),
    Item(usize),
}

impl Renderer<'_> {
    pub(super) fn render(&self) -> (String, Vec<PlanClause>) {
        let lines = self.fragment(Range {
            start: 0,
            end: self.text.len(),
        });
        // One pass over the lines collects each clause's header and item positions; a clause
        // without a rendered header is omitted from the plan.
        let mut headers = vec![None; self.clauses.len()];
        let mut items = vec![Vec::new(); self.clauses.len()];
        for (index, line) in lines.iter().enumerate() {
            match line.role {
                Role::Plain => {}
                Role::Header(clause) => {
                    headers[clause].get_or_insert(index);
                }
                Role::Item(clause) => items[clause].push(index),
            }
        }
        let plan = self
            .clauses
            .iter()
            .zip(headers)
            .zip(items)
            .filter_map(|((clause, header), items)| {
                Some(PlanClause {
                    header: header?,
                    connector: clause.kind.connector(),
                    items,
                })
            })
            .collect();
        let text = lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        (text, plan)
    }

    pub(super) fn fragment(&self, range: Range) -> Vec<Line> {
        let clauses = self
            .clauses
            .iter()
            .enumerate()
            .filter(|(_, clause)| {
                range.contains(clause.range)
                    && clause
                        .items
                        .iter()
                        .any(|index| !self.candidates[*index].conditions.is_empty())
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        let mut cursor = range.start;
        let mut output = Vec::new();
        for index in &clauses {
            let clause = &self.clauses[*index];
            if clause.range.start < cursor {
                continue;
            }
            output.extend(self.raw(Range {
                start: cursor,
                end: clause.range.start,
            }));
            output.extend(self.clause(*index));
            cursor = clause.range.end;
            if self.text[cursor..range.end]
                .chars()
                .next()
                .is_some_and(|character| character != '\n')
            {
                // The clause ended mid-line: the remainder starts a new line at the clause's
                // indentation, unless the next rendered clause begins there.
                let line_end = self.text[cursor..range.end]
                    .find('\n')
                    .map_or(range.end, |offset| cursor + offset);
                let end = clauses
                    .iter()
                    .map(|index| self.clauses[*index].range.start)
                    .find(|start| *start >= cursor)
                    .map_or(line_end, |start| start.min(line_end));
                let mut rest = self.raw(Range { start: cursor, end });
                if let Some(first) = rest.first_mut() {
                    first.text.insert_str(0, self.indentation(cursor));
                }
                output.extend(rest);
                cursor = end;
            }
        }
        output.extend(self.raw(Range {
            start: cursor,
            end: range.end,
        }));
        output
    }

    pub(super) fn clause(&self, index: usize) -> Vec<Line> {
        let clause = &self.clauses[index];
        let enclosing = gates_at(self.candidates, clause.keyword.start).collect::<Vec<_>>();
        // An OR group's header is a blank line: the `(` line already carries the enclosing
        // conjunct's item role.
        let header_text = match clause.kind {
            ClauseKind::Or => String::new(),
            kind => format!("{}{}", self.indentation(clause.keyword.start), kind.name()),
        };
        let header = Line {
            role: Role::Header(index),
            text: self.line(&header_text, &enclosing),
        };
        let indentation = |candidate: &Candidate| {
            if self.source.line_of(candidate.surface.start)
                == self.source.line_of(clause.keyword.start)
            {
                format!("{}  ", self.indentation(clause.keyword.start))
            } else {
                self.indentation(candidate.surface.start).to_string()
            }
        };
        // An all-annotated OR group keeps a `FALSE` operand so an all-inactive group still
        // parses and matches nothing.
        let synthetic = (clause.kind == ClauseKind::Or
            && clause
                .items
                .iter()
                .all(|item| !self.candidates[*item].conditions.is_empty()))
        .then(|| Line {
            role: Role::Item(index),
            text: self.line(
                &format!("{}FALSE", indentation(&self.candidates[clause.items[0]])),
                &enclosing,
            ),
        });
        let leads = synthetic.is_some();
        let items = clause
            .items
            .iter()
            .enumerate()
            .flat_map(|(position, candidate)| {
                let candidate = &self.candidates[*candidate];
                let mut lines = self.fragment(candidate.range);
                if clause.kind == ClauseKind::OrderBy
                    && position + 1 < clause.items.len()
                    && let Some(last) = lines.last_mut()
                {
                    append_trailing_comma(&mut last.text);
                }
                let connector = match clause.kind {
                    ClauseKind::Where if position > 0 => "AND ",
                    ClauseKind::Or if position > 0 || leads => "OR ",
                    _ => "",
                };
                if let Some(first) = lines.first_mut() {
                    first.role = Role::Item(index);
                    first.text = format!(
                        "{}{connector}{}",
                        indentation(candidate),
                        first.text.trim_start()
                    );
                }
                lines
            });
        std::iter::once(header)
            .chain(synthetic)
            .chain(items)
            .collect()
    }

    pub(super) fn raw(&self, range: Range) -> Vec<Line> {
        let mut cursor = range.start;
        let mut lines = Vec::new();
        while cursor < range.end {
            let end = self.text[cursor..range.end]
                .find('\n')
                .map_or(range.end, |offset| cursor + offset);
            let text = self.clean(Range { start: cursor, end });
            // A line break inside a literal is part of its value, as is the whitespace before
            // it, so the line is neither dropped when blank nor trimmed.
            let inside_literal = self
                .literals
                .iter()
                .any(|literal| literal.start < end && end < literal.end);
            if inside_literal {
                lines.push(Line {
                    role: Role::Plain,
                    text,
                });
            } else if !text.trim().is_empty() {
                let position = first_non_whitespace(self.text, cursor, end).unwrap_or(cursor);
                lines.push(Line {
                    role: Role::Plain,
                    text: self.line(
                        text.trim_end(),
                        &gates_at(self.candidates, position).collect::<Vec<_>>(),
                    ),
                });
            }
            cursor = end + 1;
        }
        lines
    }

    pub(super) fn clean(&self, range: Range) -> String {
        let mut cursor = range.start;
        let mut output = String::new();
        for comment in self
            .comments
            .iter()
            .filter(|comment| range.contains(**comment))
        {
            output.push_str(&self.text[cursor..comment.start]);
            cursor = comment.end;
        }
        output.push_str(&self.text[cursor..range.end]);
        output
    }

    /// Gates render as `-- :if $N` markers, numbered by bind slot, for the runtime.
    pub(super) fn line(&self, text: &str, conditions: &[usize]) -> String {
        conditions.iter().fold(text.to_string(), |mut line, slot| {
            let _ = write!(line, " -- :if ${}", slot + 1);
            line
        })
    }

    pub(super) fn indentation(&self, position: usize) -> &str {
        let start = self.text[..position]
            .rfind('\n')
            .map_or(0, |index| index + 1);
        let end = first_non_whitespace(self.text, start, position).unwrap_or(position);
        &self.text[start..end]
    }
}

pub(super) fn append_trailing_comma(text: &mut String) {
    let position = text.rfind(" -- :if $").unwrap_or(text.len());
    text.insert(position, ',');
}

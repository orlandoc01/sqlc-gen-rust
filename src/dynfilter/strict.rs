use sqlparser::{
    ast::{Statement, Visit},
    parser::Parser,
    tokenizer::{Token, Whitespace},
};

use super::{
    Dialect, Directive, DynFilterInfo,
    attachment::{attach_annotations, attach_switches, validate_cases},
    may_contain_annotation, number_sqlc_slices, parse_directive, references,
    render::{Renderer, blank_gated_comments},
    resolve::{self, Catalog, GatedJoin, What},
    source::{Range, Source, first_non_whitespace},
    structures::{AstStructures, Candidate, Unsupported, covered, structures_from_ast},
};

pub(super) struct Annotation {
    pub(super) range: Range,
    pub(super) line: usize,
    pub(super) standalone: bool,
    pub(super) directive: Directive,
}

pub(crate) fn parse(
    sql: &str,
    params: &[(String, usize)],
    dialect: Dialect,
    catalog: &Catalog<'_>,
) -> Result<Option<DynFilterInfo>, String> {
    if !may_contain_annotation(sql) {
        return Ok(None);
    }
    let source = Source::tokenize(sql, dialect)?;
    let annotations = annotations(&source)?;
    if annotations.is_empty() {
        return Ok(None);
    }

    let multiline = source.multiline_tokens();
    let statements = Parser::parse_sql(dialect.sqlparser(), sql)
        .map_err(|error| format!("dynamic filters could not parse query text: {error}"))?;
    let mut structures = AstStructures::default();
    let _ = statements.visit(&mut structures);
    let (mut clauses, mut candidates) = structures_from_ast(&source, structures)?;
    let references = references(
        params,
        annotations.iter().map(|annotation| &annotation.directive),
    )?;

    attach_switches(&source, &annotations, &mut clauses)?;
    attach_annotations(&source, &annotations, &mut candidates)?;
    validate_cases(&annotations, &clauses, &candidates)?;
    reject_gated_multiline_tokens(&source, &annotations, &candidates, &multiline.literals)?;
    for candidate in &mut candidates {
        candidate.conditions = candidate
            .annotation
            .map(|index| annotations[index].directive.names())
            .unwrap_or_default()
            .iter()
            .map(|name| references.arg_index_by_name[name])
            .collect();
    }
    validate_gated_joins(
        &source,
        &statements,
        &annotations,
        &candidates,
        dialect,
        catalog,
    )?;
    validate_guarded_binds(
        &source,
        &candidates,
        params,
        &references.conditional_param_numbers,
    )?;
    let text = blank_gated_comments(sql, &candidates, &multiline.comments);
    let (annotated_sql, plan) = Renderer {
        source: &source,
        text: &text,
        comments: &line_comments(&source),
        clauses: &clauses,
        candidates: &candidates,
        literals: &multiline.literals,
    }
    .render();
    let param_by_name = params
        .iter()
        .map(|(name, number)| (name.as_str(), *number))
        .collect();

    Ok(Some(DynFilterInfo {
        annotated_sql: number_sqlc_slices(&annotated_sql, dialect, &param_by_name)?,
        plan,
        conditional_param_numbers: references.conditional_param_numbers,
        flag_params: references.flag_params,
        switches: references.switches,
    }))
}

/// Directives are the `--` comment tokens whose body starts with `:`, as the dialect's own
/// tokenizer finds them, so text inside literals, quoted identifiers, block comments, or after
/// prose in an ordinary comment is never mistaken for one.
fn annotations(source: &Source<'_>) -> Result<Vec<Annotation>, String> {
    source
        .raw
        .iter()
        .filter_map(|token| match &token.token {
            Token::Whitespace(Whitespace::SingleLineComment { prefix, comment })
                if prefix == "--" && comment.trim_start().starts_with(':') =>
            {
                Some((token.span, comment))
            }
            _ => None,
        })
        .map(|(span, comment)| {
            let start = source.offset(span.start);
            let line = source.line_of(start);
            let directive = parse_directive(comment)
                .map_err(|message| format!("annotation on line {}: {message}", line + 1))?;
            Ok(Annotation {
                range: Range {
                    start,
                    end: source.line_end(start),
                },
                line,
                standalone: source.sql[source.locations[line]..start].trim().is_empty(),
                directive,
            })
        })
        .collect()
}

/// Every line comment, directive or prose, cut at its line end. The renderer drops them all:
/// the runtime reads its gate markers as line comments, so prose quoting one would gate a
/// line if it survived.
fn line_comments(source: &Source<'_>) -> Vec<Range> {
    source
        .raw
        .iter()
        .filter(|token| {
            matches!(
                token.token,
                Token::Whitespace(Whitespace::SingleLineComment { .. })
            )
        })
        .map(|token| {
            let start = source.offset(token.span.start);
            Range {
                start,
                end: source.line_end(start),
            }
        })
        .collect()
}

fn reject_gated_multiline_tokens(
    source: &Source<'_>,
    annotations: &[Annotation],
    candidates: &[Candidate],
    multiline: &[Range],
) -> Result<(), String> {
    let offending = candidates
        .iter()
        .filter_map(|candidate| candidate.annotation.map(|index| (candidate, index)))
        .find_map(|(candidate, index)| {
            multiline
                .iter()
                .find(|range| candidate.range.contains(**range))
                .map(|range| (index, range))
        });
    match offending {
        Some((index, range)) => Err(format!(
            "annotation on line {} gates a structure containing a multi-line literal or comment starting on line {}; split the value or leave the structure unconditional",
            annotations[index].line + 1,
            source.line_of(range.start) + 1
        )),
        None => Ok(()),
    }
}

/// A gated join must be droppable without changing the rest of the query: it owns its lines,
/// and every reference to the joined relation sits under at least the join's conditions.
fn validate_gated_joins(
    source: &Source<'_>,
    statements: &[Statement],
    annotations: &[Annotation],
    candidates: &[Candidate],
    dialect: Dialect,
    catalog: &Catalog<'_>,
) -> Result<(), String> {
    let gated = candidates
        .iter()
        .enumerate()
        .filter_map(|(index, candidate)| {
            Some((
                index,
                candidate,
                candidate.join.as_ref()?,
                candidate.annotation?,
            ))
        })
        .collect::<Vec<_>>();
    if gated.is_empty() {
        return Ok(());
    }
    for (_, candidate, join, index) in &gated {
        let line = annotations[*index].line + 1;
        match join.unsupported {
            Some(Unsupported::Operator(what)) => {
                return Err(format!(
                    "annotation on line {line} gates {what}; only JOIN, INNER JOIN, LEFT JOIN, and LEFT OUTER JOIN with an ON constraint take dynamic filters"
                ));
            }
            Some(Unsupported::Relation(what)) => {
                return Err(format!(
                    "annotation on line {line} gates a JOIN of {what}; dynamic filters gate joins of tables, CTEs, and aliased derived tables"
                ));
            }
            None => {}
        }
        if !owns_its_lines(source, candidate.range) {
            return Err(format!(
                "annotation on line {line} gates a JOIN that shares a line with other SQL; start the JOIN on its own line and end its line after the ON expression"
            ));
        }
        if join.wildcard {
            return Err(format!(
                "annotation on line {line} gates a JOIN in a SELECT * query, whose row shape would change; list the columns"
            ));
        }
    }
    let joins = gated
        .iter()
        .map(|(index, _, join, _)| GatedJoin {
            candidate: *index,
            relation: join.relation,
        })
        .collect::<Vec<_>>();
    for reference in resolve::references(statements, dialect, catalog, &joins)? {
        let candidate = &candidates[reference.candidate];
        let offset = source.offset(reference.at);
        if covered(candidates, offset, &candidate.conditions) {
            continue;
        }
        let annotation = &annotations[candidate
            .annotation
            .expect("gated joins carry an annotation")];
        let gate = format!(
            "-- {} @{}",
            annotation.directive.keyword(),
            annotation.directive.names().join(" @")
        );
        let alias = &reference.alias;
        let line = reference.at.line;
        return Err(match reference.what {
            What::Column(column) => format!(
                "gated JOIN column `{column}` of `{alias}` is referenced unqualified on line {line} outside a structure gated by `{gate}`; qualify it as `{alias}.{column}` and gate that structure on the same names"
            ),
            What::Relation => format!(
                "gated JOIN alias `{alias}` is referenced on line {line} outside a structure gated by `{gate}`; gate that structure on the same names as the join"
            ),
        });
    }
    Ok(())
}

fn owns_its_lines(source: &Source<'_>, range: Range) -> bool {
    let line_start = source.locations[source.line_of(range.start)];
    let rest = source.sql[range.end..source.line_end(range.end)].trim_start();
    first_non_whitespace(source.sql, line_start, range.start).is_none()
        && (rest.is_empty() || rest.starts_with("--"))
}

/// Every placeholder of an optional parameter must sit inside a structure guarded by that
/// parameter, otherwise the runtime would bind a `None`.
fn validate_guarded_binds(
    source: &Source<'_>,
    candidates: &[Candidate],
    params: &[(String, usize)],
    conditional: &[usize],
) -> Result<(), String> {
    // Mirrors the runtime: a numbered placeholder names its argument; a bare `?` takes the
    // first argument number, in field order, that no earlier placeholder has claimed.
    let mut seen = std::collections::HashSet::new();
    for token in &source.tokens {
        let Token::Placeholder(text) = &token.token else {
            continue;
        };
        let number = match text.trim_start_matches(['$', '?']).parse::<usize>() {
            Ok(number) if text.len() > 1 => number,
            _ => match params
                .iter()
                .map(|(_, number)| *number)
                .find(|number| !seen.contains(number))
            {
                Some(number) => number,
                None => continue,
            },
        };
        seen.insert(number);
        if !conditional.contains(&number) {
            continue;
        }
        if !covered(candidates, token.range.start, &[number - 1]) {
            let name = params
                .iter()
                .find(|(_, param)| *param == number)
                .map_or("?", |(name, _)| name.as_str());
            return Err(format!(
                "optional parameter `{name}` is bound on line {} outside a structure guarded by `-- :if @{name}`",
                source.line_of(token.range.start) + 1
            ));
        }
    }
    Ok(())
}

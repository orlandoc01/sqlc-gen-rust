//! Attaches each annotation to the structure it gates and validates switch cases.

use std::cmp::Reverse;

use super::{
    Directive, Switch,
    source::{Source, previous_non_whitespace},
    strict::Annotation,
    structures::{Candidate, Clause, ClauseKind},
};

pub(super) fn attach_switches(
    source: &Source<'_>,
    annotations: &[Annotation],
    clauses: &mut [Clause],
) -> Result<(), String> {
    let switches = annotations
        .iter()
        .enumerate()
        .filter(|(_, annotation)| matches!(annotation.directive, Directive::Switch(_)));
    for (index, annotation) in switches {
        let keyword_line = |clause: &Clause| source.line_of(clause.keyword.start);
        let inline = clauses.iter().position(|clause| {
            clause.kind != ClauseKind::Or
                && keyword_line(clause) == annotation.line
                && clause.keyword.end <= annotation.range.start
                && source.sql[clause.keyword.end..annotation.range.start]
                    .trim()
                    .is_empty()
        });
        let standalone = || {
            annotation.standalone.then(|| {
                clauses
                    .iter()
                    .enumerate()
                    .filter(|(_, clause)| {
                        clause.kind != ClauseKind::Or && keyword_line(clause) == annotation.line + 1
                    })
                    .min_by_key(|(_, clause)| clause.keyword.start)
                    .map(|(index, _)| index)
            })
        };
        let selected = inline.or_else(|| standalone().flatten()).ok_or_else(|| {
            format!(
                "`-- :switch` on line {} must follow a WHERE or ORDER BY keyword on its line or stand alone on the line before one",
                annotation.line + 1
            )
        })?;
        let clause = &mut clauses[selected];
        if clause.switch.is_some() {
            return Err(format!(
                "`-- :switch` on line {} is the second switch on one {} clause",
                annotation.line + 1,
                clause.kind.name()
            ));
        }
        clause.switch = Some(index);
    }
    Ok(())
}

pub(super) fn validate_cases(
    annotations: &[Annotation],
    clauses: &[Clause],
    candidates: &[Candidate],
) -> Result<(), String> {
    for clause in clauses {
        let switch = clause
            .switch
            .map(|index| match &annotations[index].directive {
                Directive::Switch(switch) => switch,
                _ => unreachable!("clause switch index points at a `:switch` annotation"),
            });
        let cases = clause
            .items
            .iter()
            .filter_map(|candidate| candidates[*candidate].annotation)
            .map(|index| &annotations[index])
            .filter_map(|annotation| match &annotation.directive {
                Directive::Case(choice) => Some((choice, annotation.line + 1)),
                _ => None,
            })
            .collect::<Vec<_>>();
        match switch {
            None => {
                if let Some((choice, line)) = cases.first() {
                    return Err(format!(
                        "`-- :case @{choice}` on line {line} is in {} without a `-- :switch`",
                        clause.kind.describe()
                    ));
                }
            }
            Some(Switch { field, choices, .. }) => {
                if let Some((choice, line)) =
                    cases.iter().find(|(choice, _)| !choices.contains(choice))
                {
                    return Err(format!(
                        "`-- :case @{choice}` on line {line} names a choice not declared by `-- :switch @{field}`"
                    ));
                }
                if let Some(choice) = choices
                    .iter()
                    .find(|choice| !cases.iter().any(|(used, _)| used == choice))
                {
                    return Err(format!(
                        "`-- :switch @{field}` declares choice `{choice}` but its {} clause has no `-- :case @{choice}`",
                        clause.kind.name()
                    ));
                }
            }
        }
    }
    Ok(())
}

pub(super) fn attach_annotations(
    source: &Source<'_>,
    annotations: &[Annotation],
    candidates: &mut [Candidate],
) -> Result<(), String> {
    let structural = annotations
        .iter()
        .enumerate()
        .filter(|(_, annotation)| !matches!(annotation.directive, Directive::Switch(_)));
    for (index, annotation) in structural {
        let targets = attachment_targets(source, annotation, candidates)?;
        for target in targets {
            let candidate = &mut candidates[target];
            if candidate.join.is_some() && matches!(annotation.directive, Directive::Case(_)) {
                return Err(format!(
                    "`-- :case` on line {} targets a JOIN; joins take `-- :if` or `-- :flag` only",
                    annotation.line + 1
                ));
            }
            if let Some(first) = candidate.annotation {
                return Err(format!(
                    "`-- {}` on line {} targets the structure already annotated by `-- {}` on line {}; a structure takes one annotation",
                    annotation.directive.keyword(),
                    annotation.line + 1,
                    annotations[first].directive.keyword(),
                    annotations[first].line + 1
                ));
            }
            candidate.annotation = Some(index);
        }
    }
    Ok(())
}

/// An inline annotation also covers every sibling structure of the same clause that lies wholly
/// on its line, so a multi-term preset can be written as `a ASC, b DESC -- :case @x`.
pub(super) fn attachment_targets(
    source: &Source<'_>,
    annotation: &Annotation,
    candidates: &[Candidate],
) -> Result<Vec<usize>, String> {
    let ambiguous = || {
        format!(
            "dynamic filter annotation on line {} has an ambiguous attachment",
            annotation.line + 1
        )
    };
    match inline_candidate(source, annotation, candidates) {
        CandidateMatch::One(primary) => {
            let group = candidates[primary].group;
            return Ok(candidates
                .iter()
                .enumerate()
                .filter(|(index, candidate)| {
                    *index == primary
                        || (candidate.group == group
                            && source.line_of(candidate.surface.start) == annotation.line
                            && source.line_of(candidate.surface.end) == annotation.line)
                })
                .map(|(index, _)| index)
                .collect());
        }
        CandidateMatch::Ambiguous => return Err(ambiguous()),
        CandidateMatch::None => {}
    }
    // A standalone line gates the structure starting on the next line; only when none starts
    // there does it fall back to the parenthesized structure it sits inside.
    let fallback = [
        standalone_candidate(source, annotation, candidates),
        contained_candidate(source, annotation, candidates),
    ]
    .into_iter()
    .find_map(|candidate| match candidate {
        CandidateMatch::None => None,
        CandidateMatch::One(index) => Some(Ok(vec![index])),
        CandidateMatch::Ambiguous => Some(Err(ambiguous())),
    });
    fallback.unwrap_or_else(|| {
        Err(if annotation.standalone {
            format!(
                "dynamic filter standalone annotation on line {} is not followed by a WHERE conjunct, OR operand, ORDER BY term, or JOIN",
                annotation.line + 1
            )
        } else {
            format!(
                "dynamic filter annotation on line {} does not attach to a WHERE conjunct, OR operand, ORDER BY term, or JOIN",
                annotation.line + 1
            )
        })
    })
}

pub(super) fn inline_candidate(
    source: &Source<'_>,
    annotation: &Annotation,
    candidates: &[Candidate],
) -> CandidateMatch {
    let matches = candidates
        .iter()
        .enumerate()
        .filter(|(_, candidate)| {
            source.line_of(candidate.surface.end) == annotation.line
                && source.sql[candidate.surface.end..annotation.range.start]
                    .trim_matches(|character: char| character.is_whitespace() || character == ',')
                    .is_empty()
        })
        .map(|(index, candidate)| (index, candidate.range.len()));
    smallest(matches)
}

pub(super) fn contained_candidate(
    source: &Source<'_>,
    annotation: &Annotation,
    candidates: &[Candidate],
) -> CandidateMatch {
    let Some(previous) = previous_non_whitespace(source.sql, annotation.range.start) else {
        return CandidateMatch::None;
    };
    if source.sql.as_bytes()[previous] != b'(' {
        return CandidateMatch::None;
    }
    let matches = candidates
        .iter()
        .enumerate()
        .filter(|(_, candidate)| {
            candidate.range.contains(annotation.range)
                && candidate.surface.start <= annotation.range.start
                && begins_parenthesized_structure(
                    source.sql,
                    candidate.surface.start,
                    annotation.range.start,
                )
        })
        .map(|(index, candidate)| (index, candidate.range.len()));
    smallest(matches)
}

pub(super) fn standalone_candidate(
    source: &Source<'_>,
    annotation: &Annotation,
    candidates: &[Candidate],
) -> CandidateMatch {
    if !annotation.standalone {
        return CandidateMatch::None;
    }
    let matches = candidates
        .iter()
        .enumerate()
        .filter(|(_, candidate)| source.line_of(candidate.surface.start) == annotation.line + 1)
        .map(|(index, candidate)| (index, candidate.surface.start, candidate.range.len()));
    outermost(matches)
}

pub(super) enum CandidateMatch {
    None,
    One(usize),
    Ambiguous,
}

/// The candidate with the smallest key; ambiguous when two share it.
fn unique_min<K: Ord>(matches: impl IntoIterator<Item = (usize, K)>) -> CandidateMatch {
    let best = matches
        .into_iter()
        .fold(None, |best, (index, key)| match best {
            None => Some((index, key, false)),
            Some((_, best_key, _)) if key < best_key => Some((index, key, false)),
            Some((best_index, best_key, tied)) => {
                let tied = tied || key == best_key;
                Some((best_index, best_key, tied))
            }
        });
    match best {
        None => CandidateMatch::None,
        Some((_, _, true)) => CandidateMatch::Ambiguous,
        Some((index, _, false)) => CandidateMatch::One(index),
    }
}

/// The shortest structure; two of equal length are ambiguous.
pub(super) fn smallest(matches: impl IntoIterator<Item = (usize, usize)>) -> CandidateMatch {
    unique_min(matches)
}

/// The structure starting first and, at the same start, extending furthest.
pub(super) fn outermost(
    matches: impl IntoIterator<Item = (usize, usize, usize)>,
) -> CandidateMatch {
    unique_min(
        matches
            .into_iter()
            .map(|(index, start, length)| (index, (start, Reverse(length)))),
    )
}

pub(super) fn begins_parenthesized_structure(sql: &str, start: usize, comment: usize) -> bool {
    sql[start..comment].trim_end().ends_with('(')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn describe(candidate: CandidateMatch) -> Option<Option<usize>> {
        match candidate {
            CandidateMatch::None => None,
            CandidateMatch::Ambiguous => Some(None),
            CandidateMatch::One(index) => Some(Some(index)),
        }
    }

    #[test]
    fn selects_a_unique_minimum_and_reports_ties() {
        assert_eq!(describe(smallest([])), None);
        assert_eq!(describe(smallest([(7, 3)])), Some(Some(7)));
        assert_eq!(describe(smallest([(1, 5), (2, 3), (3, 9)])), Some(Some(2)));
        assert_eq!(describe(smallest([(1, 3), (2, 3), (3, 9)])), Some(None));
        assert_eq!(describe(smallest([(1, 9), (2, 3), (3, 3)])), Some(None));
        // Same start: the longest wins; equal start and length is a tie.
        assert_eq!(describe(outermost([])), None);
        assert_eq!(describe(outermost([(1, 4, 2)])), Some(Some(1)));
        assert_eq!(
            describe(outermost([(1, 4, 2), (2, 4, 9), (3, 6, 30)])),
            Some(Some(2))
        );
        assert_eq!(
            describe(outermost([(1, 2, 1), (2, 4, 9), (3, 4, 9)])),
            Some(Some(1))
        );
        assert_eq!(describe(outermost([(1, 4, 9), (2, 4, 9)])), Some(None));
    }
}

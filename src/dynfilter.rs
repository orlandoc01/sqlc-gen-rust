use regex_lite::Regex;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::LazyLock;

static IF_ANNOTATION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"--\s*:if\s+[@$]\w+(?:\s+[@$]\w+)*\s*$").expect("valid :if annotation regex")
});

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FlagParam {
    pub(crate) name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DynFilterInfo {
    pub(crate) annotated_sql: String,
    pub(crate) conditional_param_numbers: Vec<usize>,
    pub(crate) flag_params: Vec<FlagParam>,
    pub(crate) ordered_arg_names: Vec<String>,
}

pub(crate) fn parse(sql: &str, params: &[(String, usize)]) -> Option<DynFilterInfo> {
    let param_by_name = params
        .iter()
        .map(|(name, number)| (name.as_str(), *number))
        .collect::<HashMap<_, _>>();
    let sql = number_sqlc_slices(sql, &param_by_name);
    let lines = sql.split('\n').collect::<Vec<_>>();
    let annotations = annotation_locations(&lines);

    let mut seen_names = HashSet::new();
    let mut references = Vec::new();
    for annotation in annotations.iter().flatten() {
        for name in parse_names(&annotation.2) {
            if seen_names.insert(name.clone()) {
                let number = param_by_name.get(name.as_str()).copied();
                references.push((name, number));
            }
        }
    }
    if references.is_empty() {
        return None;
    }

    let mut arg_index_by_name = HashMap::new();
    let mut conditional_param_numbers = BTreeSet::new();
    let mut flag_params = Vec::new();
    for (name, number) in references {
        match number {
            Some(number) => {
                arg_index_by_name.insert(name, number - 1);
                conditional_param_numbers.insert(number);
            }
            None => {
                let index = params.len() + flag_params.len();
                arg_index_by_name.insert(name.clone(), index);
                flag_params.push(FlagParam { name });
            }
        }
    }

    let suffix = |annotation: &str| {
        parse_names(annotation)
            .iter()
            .map(|name| format!("-- :if ${}", arg_index_by_name[name] + 1))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let mut output = Vec::with_capacity(lines.len());
    let mut block_suffix = None;
    let mut block_depth = 0_isize;

    for (index, line) in lines.iter().enumerate() {
        if let Some(block) = &block_suffix {
            block_depth += paren_depth(line);
            let line = match &annotations[index] {
                Some((start, _, annotation)) => format!(
                    "{} {} {}",
                    line[..*start].trim_end(),
                    suffix(annotation),
                    block,
                ),
                None => format!("{} {}", line.trim_end(), block),
            };
            output.push(line);
            if block_depth <= 0 {
                block_suffix = None;
                block_depth = 0;
            }
            continue;
        }

        let Some((start, _, annotation)) = &annotations[index] else {
            output.push((*line).to_string());
            continue;
        };
        let suffix = suffix(annotation);
        if line[..*start].trim().is_empty() {
            output.push(suffix.clone());
            if lines
                .get(index + 1)
                .is_some_and(|next_line| paren_depth(next_line) > 0)
            {
                block_suffix = Some(suffix);
            }
        } else {
            let content = line[..*start].trim_end();
            output.push(format!("{content} {suffix}"));
            let depth = paren_depth(content);
            if depth > 0 {
                block_suffix = Some(suffix);
                block_depth = depth;
            }
        }
    }

    let mut ordered_params = params.to_vec();
    ordered_params.sort_unstable_by_key(|(_, number)| *number);
    let mut ordered_arg_names = ordered_params
        .into_iter()
        .map(|(name, _)| name)
        .collect::<Vec<_>>();
    ordered_arg_names.extend(flag_params.iter().map(|flag| flag.name.clone()));

    Some(DynFilterInfo {
        annotated_sql: output.join("\n"),
        conditional_param_numbers: conditional_param_numbers.into_iter().collect(),
        flag_params,
        ordered_arg_names,
    })
}

fn annotation_locations(lines: &[&str]) -> Vec<Option<(usize, usize, String)>> {
    let mut state = LexState::default();
    lines
        .iter()
        .map(|line| {
            let annotation = (!state.open())
                .then(|| IF_ANNOTATION.find(line))
                .flatten()
                .map(|matched| {
                    (
                        matched.start(),
                        matched.end(),
                        line[matched.start()..matched.end()].to_string(),
                    )
                });
            state = line_end_state(line, state.clone());
            annotation
        })
        .collect()
}

fn parse_names(annotation: &str) -> Vec<String> {
    annotation
        .split_whitespace()
        .filter_map(|part| {
            part.strip_prefix('@')
                .or_else(|| part.strip_prefix('$'))
                .map(str::to_string)
        })
        .collect()
}

fn number_sqlc_slices(sql: &str, param_by_name: &HashMap<&str, usize>) -> String {
    let mut output = String::with_capacity(sql.len());
    let mut rest = sql;
    while let Some(start) = rest.find("/*SLICE:") {
        let (before, marker) = rest.split_at(start);
        output.push_str(before);
        let Some(end) = marker.find("*/?") else {
            output.push_str(marker);
            return output;
        };
        let name = &marker[8..end];
        if let Some(number) = param_by_name.get(name) {
            output.push_str("/*SLICE:");
            output.push_str(name);
            output.push_str("*/?");
            output.push_str(&number.to_string());
        } else {
            output.push_str(&marker[..end + 3]);
        }
        rest = &marker[end + 3..];
    }
    output.push_str(rest);
    output
}

fn paren_depth(line: &str) -> isize {
    line.bytes()
        .map(|byte| match byte {
            b'(' => 1,
            b')' => -1,
            _ => 0,
        })
        .sum()
}

#[derive(Clone, Default)]
struct LexState {
    quote: Option<u8>,
    comment_depth: usize,
    dollar: Option<String>,
}

impl LexState {
    fn open(&self) -> bool {
        self.quote.is_some() || self.comment_depth > 0 || self.dollar.is_some()
    }
}

fn line_end_state(line: &str, mut state: LexState) -> LexState {
    let bytes = line.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if let Some(delimiter) = &state.dollar {
            let Some(end) = line[index..].find(delimiter) else {
                return state;
            };
            index += end + delimiter.len();
            state.dollar = None;
            continue;
        }
        if state.comment_depth > 0 {
            while index < bytes.len() && state.comment_depth > 0 {
                if bytes[index..].starts_with(b"/*") {
                    state.comment_depth += 1;
                    index += 2;
                } else if bytes[index..].starts_with(b"*/") {
                    state.comment_depth -= 1;
                    index += 2;
                } else {
                    index += 1;
                }
            }
            continue;
        }
        if let Some(quote) = state.quote {
            if quote == b'[' {
                let Some(end) = line[index..].find(']') else {
                    return state;
                };
                index += end + 1;
                state.quote = None;
                continue;
            }
            let (end, closed) = quote_end(bytes, index, quote, quote != b'`');
            if !closed {
                return state;
            }
            index = end;
            state.quote = None;
            continue;
        }

        match bytes[index..] {
            [b'-', b'-', ..] => return state,
            [b'/', b'*', ..] => {
                state.comment_depth = 1;
                index += 2;
            }
            [quote @ (b'\'' | b'"' | b'`'), ..] => {
                state.quote = Some(quote);
                index += 1;
            }
            [b'[', ..] => {
                state.quote = Some(b'[');
                index += 1;
            }
            [b'$', ..] => {
                if let Some(delimiter) = dollar_delimiter(line, index) {
                    index += delimiter.len();
                    state.dollar = Some(delimiter);
                } else {
                    index += 1;
                }
            }
            _ => index += 1,
        }
    }
    state
}

fn quote_end(bytes: &[u8], mut index: usize, quote: u8, backslash: bool) -> (usize, bool) {
    while index < bytes.len() {
        if backslash && bytes[index] == b'\\' {
            index += 2;
            continue;
        }
        if bytes[index] != quote {
            index += 1;
            continue;
        }
        if bytes.get(index + 1) == Some(&quote) {
            index += 2;
            continue;
        }
        return (index + 1, true);
    }
    (bytes.len(), false)
}

fn dollar_delimiter(text: &str, start: usize) -> Option<String> {
    let bytes = text.as_bytes();
    let mut index = start + 1;
    while let Some(byte) = bytes.get(index) {
        if *byte == b'$' {
            return Some(text[start..=index].to_string());
        }
        let is_identifier = *byte == b'_'
            || byte.is_ascii_alphabetic()
            || (index > start + 1 && byte.is_ascii_digit());
        if !is_identifier {
            return None;
        }
        index += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(names: &[&str]) -> Vec<(String, usize)> {
        names
            .iter()
            .enumerate()
            .map(|(index, name)| (name.to_string(), index + 1))
            .collect()
    }

    #[test]
    fn ignores_sql_without_annotations() {
        assert_eq!(parse("SELECT * FROM t WHERE a = $1", &params(&["a"])), None);
    }

    #[test]
    fn rewrites_inline_annotations_and_collects_conditionals() {
        let info = parse(
            "SELECT * FROM t\nWHERE a = $1\n  AND b = $2 -- :if @b @c",
            &params(&["a", "b", "c"]),
        )
        .unwrap();

        assert_eq!(info.conditional_param_numbers, [2, 3]);
        assert_eq!(
            info.annotated_sql,
            "SELECT * FROM t\nWHERE a = $1\n  AND b = $2 -- :if $2 -- :if $3"
        );
        assert_eq!(info.ordered_arg_names, ["a", "b", "c"]);
    }

    #[test]
    fn appends_flag_parameters_in_appearance_order() {
        let info = parse(
            "SELECT * FROM t\nORDER BY\n  id ASC -- :if @id_asc\n  id DESC -- :if $id_desc",
            &params(&["a"]),
        )
        .unwrap();

        assert_eq!(
            info.flag_params,
            [
                FlagParam {
                    name: "id_asc".into()
                },
                FlagParam {
                    name: "id_desc".into()
                }
            ]
        );
        assert_eq!(info.ordered_arg_names, ["a", "id_asc", "id_desc"]);
        assert!(info.annotated_sql.contains("-- :if $2"));
        assert!(info.annotated_sql.contains("-- :if $3"));
    }

    #[test]
    fn propagates_block_conditions() {
        let info = parse(
            "SELECT * FROM t\nWHERE a = $1\n  AND EXISTS ( -- :if @has_orders\n    SELECT 1\n    FROM orders\n  )",
            &params(&["a"]),
        )
        .unwrap();

        assert_eq!(
            info.annotated_sql,
            "SELECT * FROM t\nWHERE a = $1\n  AND EXISTS ( -- :if $2\n    SELECT 1 -- :if $2\n    FROM orders -- :if $2\n  ) -- :if $2"
        );
    }

    #[test]
    fn standalone_block_conditions_keep_the_marker() {
        let info = parse(
            "SELECT * FROM t\n-- :if @enabled\n  AND EXISTS (\n    SELECT 1\n  )",
            &params(&[]),
        )
        .unwrap();

        assert_eq!(
            info.annotated_sql,
            "SELECT * FROM t\n-- :if $1\n  AND EXISTS ( -- :if $1\n    SELECT 1 -- :if $1\n  ) -- :if $1"
        );
    }

    #[test]
    fn numbers_slice_markers() {
        let info = parse(
            "SELECT * FROM t WHERE id IN (/*SLICE:team-ids*/?) -- :if @active",
            &params(&["active", "team-ids"]),
        )
        .unwrap();

        assert!(info.annotated_sql.contains("/*SLICE:team-ids*/?2"));
    }

    #[test]
    fn ignores_annotation_text_on_a_multiline_literal_continuation() {
        assert_eq!(
            parse(
                "SELECT 'hello\n-- :if @name\nworld' FROM t WHERE name = $1",
                &params(&["name"])
            ),
            None
        );
    }
}

//! The directive grammar: `-- :if @a @b`, `-- :flag @x`, `-- :case @choice`, and
//! `-- :switch @field a b default=a`.

use std::collections::HashSet;

use super::Switch;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Directive {
    If(Vec<String>),
    Flag(Vec<String>),
    Case(String),
    Switch(Switch),
}

impl Directive {
    pub(crate) fn keyword(&self) -> &'static str {
        match self {
            Self::If(_) => ":if",
            Self::Flag(_) => ":flag",
            Self::Case(_) => ":case",
            Self::Switch(_) => ":switch",
        }
    }

    /// Slot names a structure-level directive gates on; a switch gates nothing itself.
    pub(crate) fn names(&self) -> &[String] {
        match self {
            Self::If(names) | Self::Flag(names) => names,
            Self::Case(choice) => std::slice::from_ref(choice),
            Self::Switch(_) => &[],
        }
    }
}

/// Parses the body of a `--` comment whose first non-blank character is `:`.
pub(crate) fn parse_directive(comment: &str) -> Result<Directive, String> {
    let body = comment
        .trim_start()
        .strip_prefix(':')
        .ok_or_else(|| "annotation comment does not start with `:`".to_string())?;
    let keyword_len = body
        .bytes()
        .take_while(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
        .count();
    let (keyword, rest) = body.split_at(keyword_len);
    if keyword.is_empty() {
        return Err(
            "`-- :` is missing a keyword; expected `:if`, `:flag`, `:switch`, or `:case`"
                .to_string(),
        );
    }
    let args = rest.split_whitespace().collect::<Vec<_>>();
    match keyword {
        "if" => parse_names(keyword, &args).map(Directive::If),
        "flag" => {
            let names = parse_names(keyword, &args)?;
            names
                .iter()
                .try_for_each(|name| crate::validate_field_name(name))?;
            Ok(Directive::Flag(names))
        }
        "case" => match parse_names(keyword, &args)?.as_slice() {
            [choice] => {
                crate::validate_type_name(choice)?;
                Ok(Directive::Case(choice.clone()))
            }
            _ => Err("`-- :case` takes exactly one `@choice`".to_string()),
        },
        "switch" => parse_switch(&args).map(Directive::Switch),
        _ => Err(format!(
            "unknown annotation `-- :{keyword}`; expected `:if`, `:flag`, `:switch`, or `:case`"
        )),
    }
}

fn parse_names(keyword: &str, args: &[&str]) -> Result<Vec<String>, String> {
    let names = args
        .iter()
        .map(|part| strip_sigil(part).map(str::to_string))
        .collect::<Option<Vec<_>>>()
        .filter(|names| !names.is_empty())
        .ok_or_else(|| format!("`-- :{keyword}` expects one or more `@name` arguments"))?;
    Ok(names)
}

fn parse_switch(args: &[&str]) -> Result<Switch, String> {
    const USAGE: &str = "`-- :switch @field choice choice [choice ...] default=choice`";
    let [field, rest @ ..] = args else {
        return Err(format!("malformed switch; expected {USAGE}"));
    };
    let field = strip_sigil(field)
        .ok_or_else(|| format!("switch field must be written `@field`; expected {USAGE}"))?;
    let (defaults, choices): (Vec<&str>, Vec<&str>) =
        rest.iter().partition(|part| part.starts_with("default="));
    let default = match defaults.as_slice() {
        [default] => &default["default=".len()..],
        _ => {
            return Err(format!(
                "switch `@{field}` needs exactly one `default=choice`"
            ));
        }
    };
    if choices.len() < 2 {
        return Err(format!("switch `@{field}` needs at least two choices"));
    }
    if let Some(bad) = choices.iter().find(|choice| !is_identifier(choice)) {
        return Err(format!(
            "switch `@{field}` choice `{bad}` is not a bare identifier"
        ));
    }
    let mut seen = HashSet::new();
    if let Some(duplicate) = choices.iter().find(|choice| !seen.insert(**choice)) {
        return Err(format!(
            "switch `@{field}` declares choice `{duplicate}` twice"
        ));
    }
    if !choices.contains(&default) {
        return Err(format!(
            "switch `@{field}` default `{default}` is not one of its choices"
        ));
    }
    crate::validate_field_name(field)?;
    choices
        .iter()
        .try_for_each(|choice| crate::validate_type_name(choice))?;
    Ok(Switch {
        field: field.to_string(),
        choices: choices.into_iter().map(str::to_string).collect(),
        default: default.to_string(),
    })
}

fn strip_sigil(part: &str) -> Option<&str> {
    part.strip_prefix('@')
        .or_else(|| part.strip_prefix('$'))
        .filter(|name| is_identifier(name))
}

fn is_identifier(text: &str) -> bool {
    let mut bytes = text.bytes();
    bytes
        .next()
        .is_some_and(|first| first == b'_' || first.is_ascii_alphabetic())
        && bytes.all(|byte| byte == b'_' || byte.is_ascii_alphanumeric())
}

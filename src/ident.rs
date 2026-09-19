use convert_case::{Case, Casing as _};

pub(crate) fn normalize_str(value: &str) -> String {
    use regex_lite::Regex;
    use std::sync::LazyLock;
    static IDENT_PATTERN: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"[^a-zA-Z0-9_]"#).unwrap());

    let value = value.replace("-", "_");
    let value = value.replace(":", "_");
    let value = value.replace("/", "_");
    let value = IDENT_PATTERN.replace_all(&value, "");
    value.to_string()
}

/// Rejects names whose Rust spelling would be empty, start with a digit, or (for type-level
/// names) be a reserved word, before any `format_ident!` can panic on them.
pub(crate) fn validate_field_name(name: &str) -> Result<(), String> {
    checked_spelling(name, Case::Snake).map(drop)
}

pub(crate) fn validate_type_name(name: &str) -> Result<(), String> {
    let spelling = checked_spelling(name, Case::Pascal)?;
    syn::parse_str::<syn::Ident>(&spelling)
        .map(drop)
        .map_err(|_| format!("`{name}` would become the reserved Rust identifier `{spelling}`"))
}

fn checked_spelling(name: &str, case: Case) -> Result<String, String> {
    let spelling = normalize_str(name).to_case(case);
    if spelling.chars().all(|c| c == '_') || spelling.starts_with(|c: char| c.is_ascii_digit()) {
        return Err(format!(
            "`{name}` cannot be turned into a Rust identifier (it normalizes to `{spelling}`)"
        ));
    }
    Ok(spelling)
}

pub(crate) fn value_ident(ident: &str) -> syn::Ident {
    let ident = normalize_str(ident).to_case(Case::Pascal);
    quote::format_ident!("{}", ident)
}

pub(crate) fn field_ident(ident: &str) -> syn::Ident {
    let ident = normalize_str(ident).to_case(Case::Snake);
    const RAW_IDENTIFIER_EXCEPTIONS: &[&str] = &["crate", "self", "super", "Self"];
    const KEYWORDS: &[&str] = &[
        "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn",
        "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref",
        "return", "self", "Self", "static", "struct", "super", "trait", "true", "type", "union",
        "unsafe", "use", "where", "while", "async", "await", "dyn", "abstract", "become", "box",
        "do", "final", "macro", "override", "priv", "typeof", "unsized", "virtual", "yield", "try",
        "gen",
    ];

    if RAW_IDENTIFIER_EXCEPTIONS.contains(&ident.as_str()) {
        quote::format_ident!("{}_", ident)
    } else if KEYWORDS.contains(&ident.as_str()) {
        syn::Ident::new_raw(&ident, proc_macro2::Span::call_site())
    } else {
        quote::format_ident!("{}", ident)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_keyword_field_identifiers() {
        assert_eq!(field_ident("type").to_string(), "r#type");
        assert_eq!(field_ident("union").to_string(), "r#union");
        assert_eq!(field_ident("crate").to_string(), "crate_");
        assert_eq!(field_ident("id").to_string(), "id");
    }
}

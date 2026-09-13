use convert_case::{Case, Casing as _};
use prost::Message as _;
use std::io::{Read as _, Write};

pub(crate) mod plugin {
    include!(concat!(env!("OUT_DIR"), "/plugin.rs"));
}
pub(crate) mod db_crates;
pub(crate) mod dynfilter;
pub(crate) mod path_map;
pub(crate) mod query;
#[cfg(test)]
mod dynfilter_runtime {
    include!("db_crates/dynfilter_runtime.rs");
}
use query::{Query, ReturningRows, RsType, collect_enums};
pub trait StackError: std::error::Error {
    /// format each error stack
    fn format_stack(&self, layer: usize, buf: &mut Vec<String>);
    /// next error
    fn next(&self) -> Option<&dyn StackError>;

    /// last error
    fn last(&self) -> &dyn StackError
    where
        Self: Sized,
    {
        let Some(mut result) = self.next() else {
            return self;
        };
        while let Some(err) = result.next() {
            result = err;
        }
        result
    }
}

pub(crate) trait StackErrorResult<T, E> {
    fn stacked(self) -> Result<T, E>;
}

pub trait StackErrorExt: StackError {
    fn stack_error(&self) -> Vec<String>
    where
        Self: Sized,
    {
        let mut buf = Vec::new();
        let mut layer = 0;
        let mut current: &dyn StackError = self;

        loop {
            current.format_stack(layer, &mut buf);
            match current.next() {
                Some(next) => {
                    current = next;
                    layer += 1;
                }
                None => break,
            }
        }

        buf
    }
}

impl<E: StackError> StackErrorExt for E {}

#[derive(Debug)]
pub enum Error {
    Io {
        source: std::io::Error,
        location: &'static std::panic::Location<'static>,
    },
    ProstDecode {
        source: prost::DecodeError,
        location: &'static std::panic::Location<'static>,
    },
    Json {
        source: serde_json::Error,
        location: &'static std::panic::Location<'static>,
    },
    QueryError {
        source: query::QueryError,
        location: &'static std::panic::Location<'static>,
    },
    Any {
        source: Box<dyn std::error::Error + 'static>,
        location: &'static std::panic::Location<'static>,
    },
}

impl Error {
    fn location(&self) -> &'static std::panic::Location<'static> {
        match self {
            Error::Io { location, .. } => location,
            Error::ProstDecode { location, .. } => location,
            Error::Json { location, .. } => location,
            Error::Any { location, .. } => location,
            Error::QueryError { location, .. } => location,
        }
    }

    #[track_caller]
    fn any(source: Box<dyn std::error::Error + 'static>) -> Self {
        Error::Any {
            source,
            location: std::panic::Location::caller(),
        }
    }
}

impl From<std::io::Error> for Error {
    #[track_caller]
    fn from(value: std::io::Error) -> Self {
        Self::Io {
            source: value,
            location: std::panic::Location::caller(),
        }
    }
}

impl From<prost::DecodeError> for Error {
    #[track_caller]
    fn from(value: prost::DecodeError) -> Self {
        Self::ProstDecode {
            source: value,
            location: std::panic::Location::caller(),
        }
    }
}

impl From<serde_json::Error> for Error {
    #[track_caller]
    fn from(value: serde_json::Error) -> Self {
        Self::Json {
            source: value,
            location: std::panic::Location::caller(),
        }
    }
}

impl From<query::QueryError> for Error {
    #[track_caller]
    fn from(value: query::QueryError) -> Self {
        Self::QueryError {
            source: value,
            location: std::panic::Location::caller(),
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io { source, .. } => source.fmt(f),
            Error::ProstDecode { source, .. } => source.fmt(f),
            Error::Json { source, .. } => source.fmt(f),
            Error::Any { source, .. } => source.fmt(f),
            Error::QueryError { source, .. } => source.fmt(f),
        }
    }
}

impl StackError for Error {
    fn format_stack(&self, layer: usize, buf: &mut Vec<String>) {
        let location = self.location();
        let message = format!(
            "{}:{} , at {}:{}",
            layer,
            self,
            location.file(),
            location.line()
        );
        buf.push(message);
    }

    fn next(&self) -> Option<&dyn StackError> {
        match self {
            Error::QueryError { source, .. } => Some(source),
            _ => None,
        }
    }

    fn last(&self) -> &dyn StackError
    where
        Self: Sized,
    {
        let Some(mut result) = self.next() else {
            return self;
        };
        while let Some(err) = result.next() {
            result = err;
        }
        result
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io { source, .. } => Some(source),
            Error::ProstDecode { source, .. } => Some(source),
            Error::Json { source, .. } => Some(source),
            Error::QueryError { source, .. } => Some(source),
            Error::Any { source, .. } => Some(source.as_ref()),
        }
    }
}

fn deserialize_codegen_request(data: &[u8]) -> Result<plugin::GenerateRequest, prost::DecodeError> {
    plugin::GenerateRequest::decode(data)
}

fn serialize_codegen_response(response: &plugin::GenerateResponse) -> Vec<u8> {
    response.encode_to_vec()
}

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

#[derive(Debug, Clone, serde::Deserialize, Default)]
#[serde(default)]
struct OverrideType {
    /// Override db type
    db_type: Option<String>,
    /// Override column name
    column: Option<String>,
    /// Override Rust type
    rs_type: String,
    /// Rust type's slice if have
    rs_slice: Option<String>,
    /// Marker is copy cheap
    copy_cheap: bool,
}

#[derive(Debug, serde::Deserialize)]
#[serde(default)]
struct Config {
    output: String,
    db_crate: db_crates::DbCrate,
    api: Option<String>,
    query_parameter_limit: usize,
    overrides: Vec<OverrideType>,
    debug: bool,
    #[serde(flatten)]
    return_row_attributes: query::ReturnRowAttributes,
    enum_derives: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            output: "queries.rs".into(),
            db_crate: Default::default(),
            api: None,
            query_parameter_limit: 1,
            overrides: Default::default(),
            debug: false,
            return_row_attributes: Default::default(),
            enum_derives: Vec::new(),
        }
    }
}

impl Config {
    fn from_option(buf: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(buf)
    }

    fn validate(&self, queries: &[Query]) -> Result<(), Error> {
        if self.api.as_deref() == Some("builder") {
            return Err(Error::any(
                "the builder API was removed; only params_struct is generated".into(),
            ));
        }

        if let Some(query) = queries
            .iter()
            .find(|query| !self.db_crate.supports(query.annotation))
        {
            return Err(Error::any(
                format!(
                    "params_struct does not support {} with {} ({}).",
                    query.annotation, self.db_crate, query.query_name
                )
                .into(),
            ));
        }

        Ok(())
    }
}

fn generate_comment(sqlc_version: &str) -> String {
    format!(
        r"//! Code generated by {}. SHOULD NOT EDIT.
//! sqlc version: {}
//! {} version: v{}",
        env!("CARGO_PKG_NAME"),
        sqlc_version,
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION"),
    )
}

pub fn try_main() -> Result<(), Error> {
    let mut stdin = std::io::stdin().lock();
    let mut buffer = Vec::new();
    stdin.read_to_end(&mut buffer)?;

    let request = deserialize_codegen_request(&buffer)?;
    let config = if request.plugin_options.is_empty() {
        Config::default()
    } else {
        Config::from_option(&request.plugin_options)?
    };

    let mut db_type = config.db_crate.db_type_map();
    for override_type in &config.overrides {
        let owned_type = syn::parse_str::<syn::Type>(&override_type.rs_type)
            .map_err(|e| Error::any(e.into()))?;
        let slice_type = override_type
            .rs_slice
            .as_deref()
            .map(syn::parse_str::<syn::Type>)
            .transpose()
            .map_err(|e| Error::any(e.into()))?;

        match (
            override_type.db_type.as_deref(),
            override_type.column.as_deref(),
        ) {
            (None, Some(column)) => {
                db_type.insert_column_type(
                    column,
                    RsType::new(
                        owned_type.clone(),
                        slice_type.clone(),
                        override_type.copy_cheap,
                    ),
                );
            }
            (Some(db_type_name), None) => {
                db_type.insert_db_type(
                    db_type_name,
                    RsType::new(
                        owned_type.clone(),
                        slice_type.clone(),
                        override_type.copy_cheap,
                    ),
                );
            }

            (Some(_), Some(_)) => {
                let message = "Cannot override both db_type and column name at the same time.";
                return Err(Error::any(message.into()));
            }
            (None, None) => {
                let message = "Must override either db_type or column name.";
                return Err(Error::any(message.into()));
            }
        }
    }

    let enum_derives = config
        .enum_derives
        .iter()
        .map(|d| syn::parse_str::<syn::Path>(d).map_err(|e| Error::any(e.into())))
        .collect::<Result<Vec<_>, _>>()?;

    let mut defined_enums = request
        .catalog
        .as_ref()
        .map(collect_enums)
        .unwrap_or_default();

    for e in &mut defined_enums {
        e.derives = enum_derives.clone();
    }

    for e in &defined_enums {
        db_type.insert_db_type(
            &e.name,
            RsType::new(
                syn::Type::Path(syn::TypePath {
                    attrs: Vec::new(),
                    qself: None,
                    path: e.ident().clone().into(),
                }),
                None,
                true,
            ),
        );
    }

    let returning_rows = request
        .queries
        .iter()
        .map(|q| {
            ReturningRows::from_query(
                &db_type,
                &config.return_row_attributes,
                request.catalog.as_ref(),
                q,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;

    let mut queries = request
        .queries
        .iter()
        .map(|q| Query::from_query(&db_type, q))
        .collect::<Result<Vec<_>, _>>()?;

    let static_slices = config.db_crate.apply_static_slices();
    for query in &mut queries {
        query.apply_dynfilter();
        if static_slices && query.dynfilter().is_none() {
            query.apply_static_slices();
        }
    }

    config.validate(&queries)?;

    let enums_ts = defined_enums
        .iter()
        .map(|e| config.db_crate.defined_enum(e))
        .collect::<Vec<_>>();
    let enums_tt = quote::quote! {#(#enums_ts)*};
    let embedded_tables_tt = db_crates::make_embedded_tables(&returning_rows)?;

    let init_tt = config.db_crate.init();
    let queries_tt = config.db_crate.generate_queries(
        &returning_rows,
        &queries,
        config.query_parameter_limit,
    )?;
    let tt = quote::quote! {
        #init_tt
        #enums_tt
        #embedded_tables_tt
        #queries_tt
    };
    let mut response = plugin::GenerateResponse::default();
    let ast = syn::parse2(tt).map_err(|e| Error::any(e.into()))?;
    let contents = format!(
        "{}\n\n{}",
        generate_comment(&request.sqlc_version),
        prettyplease::unparse(&ast)
    );
    let query_file = plugin::File {
        name: config.output,
        contents: contents.into(),
    };
    response.files.push(query_file);

    if config.debug {
        let req_txt = format!("{request:#?}");
        response.files.push(plugin::File {
            name: "input.txt".into(),
            contents: req_txt.into_bytes(),
        });

        response.files.push(plugin::File {
            name: "input.bin".into(),
            contents: buffer,
        });
    }

    let serialized_response = serialize_codegen_response(&response);

    std::io::stdout().write_all(&serialized_response)?;

    Ok(())
}

#[cfg(test)]
mod identifier_tests {
    use super::*;

    #[test]
    fn escapes_keyword_field_identifiers() {
        assert_eq!(field_ident("type").to_string(), "r#type");
        assert_eq!(field_ident("union").to_string(), "r#union");
        assert_eq!(field_ident("crate").to_string(), "crate_");
        assert_eq!(field_ident("id").to_string(), "id");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_removed_db_crates_and_legacy_api() {
        let error = Config::from_option(br#"{"db_crate":"postgres"}"#).unwrap_err();
        assert!(
            error.to_string().contains("is not supported yet"),
            "{error}"
        );

        let config = Config::from_option(br#"{"api":"builder"}"#).unwrap();
        assert_eq!(
            config.validate(&[]).unwrap_err().to_string(),
            "the builder API was removed; only params_struct is generated"
        );

        assert_eq!(
            Config::from_option(br#"{"api":"params_struct"}"#)
                .unwrap()
                .api
                .as_deref(),
            Some("params_struct")
        );
    }

    #[test]
    fn parses_rusqlite_db_crate() {
        assert!(matches!(
            Config::from_option(br#"{"db_crate":"rusqlite"}"#)
                .unwrap()
                .db_crate,
            db_crates::DbCrate::Rusqlite
        ));
        assert!(matches!(
            Config::from_option(br#"{"db_crate":"tokio-postgres"}"#)
                .unwrap()
                .db_crate,
            db_crates::DbCrate::TokioPostgres(db_crates::TokioPostgres::Tokio)
        ));
        assert!(matches!(
            Config::from_option(br#"{"db_crate":"deadpool-postgres"}"#)
                .unwrap()
                .db_crate,
            db_crates::DbCrate::TokioPostgres(db_crates::TokioPostgres::Deadpool)
        ));
    }

    #[test]
    fn rejects_unsupported_params_struct_annotations() {
        let mut query = Query::from_query(
            &db_crates::DbCrate::Rusqlite.db_type_map(),
            &plugin::Query {
                text: "DELETE FROM authors".to_string(),
                name: "DeleteAuthors".to_string(),
                cmd: ":execresult".to_string(),
                columns: Vec::new(),
                params: Vec::new(),
                comments: Vec::new(),
                filename: String::new(),
                insert_into_table: None,
            },
        )
        .unwrap();
        let config = Config::from_option(br#"{"db_crate":"rusqlite"}"#).unwrap();
        assert_eq!(
            config
                .validate(std::slice::from_ref(&query))
                .unwrap_err()
                .to_string(),
            "params_struct does not support :execresult with rusqlite (DeleteAuthors)."
        );

        query.annotation = query::Annotation::CopyFrom;
        assert_eq!(
            config
                .validate(std::slice::from_ref(&query))
                .unwrap_err()
                .to_string(),
            "params_struct does not support :copyfrom with rusqlite (DeleteAuthors)."
        );

        query.annotation = query::Annotation::ExecLastId;
        let config = Config::from_option(br#"{"db_crate":"tokio-postgres"}"#).unwrap();
        assert_eq!(
            config
                .validate(std::slice::from_ref(&query))
                .unwrap_err()
                .to_string(),
            "params_struct does not support :execlastid with tokio-postgres (DeleteAuthors)."
        );

        let config = Config::from_option(br#"{"db_crate":"deadpool-postgres"}"#).unwrap();
        assert_eq!(
            config
                .validate(std::slice::from_ref(&query))
                .unwrap_err()
                .to_string(),
            "params_struct does not support :execlastid with deadpool-postgres (DeleteAuthors)."
        );
    }
}

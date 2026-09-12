use crate::{
    db_crates::params_common::{self, GeneratedFunction},
    query::{Annotation, Query},
};

pub(super) fn generated_functions(query: &Query) -> Vec<GeneratedFunction> {
    let name = params_common::query_function_ident(query);
    let function = |ident, helper| GeneratedFunction { ident, helper };
    if query.dynfilter().is_some() {
        let helper = function(quote::format_ident!("{name}_query"), "dynamic query helper");
        return match query.annotation {
            Annotation::One => vec![
                helper,
                function(name.clone(), "query function"),
                function(quote::format_ident!("{name}_opt"), "optional query helper"),
            ],
            Annotation::Many => vec![
                helper,
                function(name.clone(), "query function"),
                function(quote::format_ident!("{name}_stream"), "stream helper"),
            ],
            Annotation::Exec | Annotation::ExecRows | Annotation::ExecResult => {
                vec![helper, function(name, "query function")]
            }
            Annotation::ExecLastId
            | Annotation::BatchExec
            | Annotation::BatchMany
            | Annotation::BatchOne
            | Annotation::CopyFrom => Vec::new(),
        };
    }

    let prepare = function(quote::format_ident!("prepare_{name}"), "prepare helper");
    match query.annotation {
        Annotation::One => vec![
            prepare,
            function(name.clone(), "query function"),
            function(quote::format_ident!("{name}_with"), "with helper"),
            function(quote::format_ident!("{name}_opt"), "optional query helper"),
            function(
                quote::format_ident!("{name}_opt_with"),
                "optional with helper",
            ),
        ],
        Annotation::Many => vec![
            prepare,
            function(name.clone(), "query function"),
            function(quote::format_ident!("{name}_with"), "with helper"),
            function(quote::format_ident!("{name}_stream"), "stream helper"),
            function(
                quote::format_ident!("{name}_stream_with"),
                "stream with helper",
            ),
        ],
        Annotation::Exec | Annotation::ExecRows | Annotation::ExecResult => vec![
            prepare,
            function(name.clone(), "query function"),
            function(quote::format_ident!("{name}_with"), "with helper"),
        ],
        Annotation::ExecLastId
        | Annotation::BatchExec
        | Annotation::BatchMany
        | Annotation::BatchOne
        | Annotation::CopyFrom => Vec::new(),
    }
}

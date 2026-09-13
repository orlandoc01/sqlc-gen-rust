use crate::{
    db_crates::{
        Postgres,
        params_common::{self, GeneratedFunction},
    },
    query::{Annotation, Query},
};

pub(super) fn generated_functions(backend: Postgres, query: &Query) -> Vec<GeneratedFunction> {
    fn function(ident: syn::Ident, helper: impl Into<String>) -> GeneratedFunction {
        GeneratedFunction {
            ident,
            helper: helper.into(),
        }
    }

    let name = params_common::query_function_ident(query);
    if query.dynfilter().is_some() {
        let helper = function(quote::format_ident!("{name}_query"), "dynamic query helper");
        return match query.annotation {
            Annotation::One => vec![
                helper,
                function(name.clone(), "query function"),
                function(quote::format_ident!("{name}_opt"), "optional query helper"),
            ],
            Annotation::Many => {
                let suffix = backend.many_iterator_suffix();
                vec![
                    helper,
                    function(name.clone(), "query function"),
                    function(
                        quote::format_ident!("{name}_{suffix}"),
                        format!("{suffix} helper"),
                    ),
                ]
            }
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
        Annotation::Many => {
            let suffix = backend.many_iterator_suffix();
            vec![
                prepare,
                function(name.clone(), "query function"),
                function(quote::format_ident!("{name}_with"), "with helper"),
                function(
                    quote::format_ident!("{name}_{suffix}"),
                    format!("{suffix} helper"),
                ),
                function(
                    quote::format_ident!("{name}_{suffix}_with"),
                    format!("{suffix} with helper"),
                ),
            ]
        }
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

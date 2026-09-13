use crate::{
    db_crates::{
        DbCrate, Postgres,
        params_common::{self, GeneratedFunction},
    },
    query::{Annotation, Query},
};

pub(super) fn generated_functions(backend: Postgres, query: &Query) -> Vec<GeneratedFunction> {
    if !DbCrate::Postgres(backend).supports(query.annotation) {
        return Vec::new();
    }

    let name = params_common::query_function_ident(query);
    if query.dynfilter().is_some() {
        let helper = GeneratedFunction {
            ident: quote::format_ident!("{name}_query"),
            helper: "dynamic query helper",
        };
        return match query.annotation {
            Annotation::One => vec![
                helper,
                GeneratedFunction {
                    ident: name.clone(),
                    helper: "query function",
                },
                GeneratedFunction {
                    ident: quote::format_ident!("{name}_opt"),
                    helper: "optional query helper",
                },
            ],
            Annotation::Many => {
                let suffix = backend.many_iterator_suffix();
                vec![
                    helper,
                    GeneratedFunction {
                        ident: name.clone(),
                        helper: "query function",
                    },
                    GeneratedFunction {
                        ident: quote::format_ident!("{name}_{suffix}"),
                        helper: backend.many_iterator_helpers().0,
                    },
                ]
            }
            Annotation::Exec | Annotation::ExecRows | Annotation::ExecResult => {
                vec![
                    helper,
                    GeneratedFunction {
                        ident: name,
                        helper: "query function",
                    },
                ]
            }
            _ => unreachable!("unsupported annotation was filtered"),
        };
    }

    let prepare = GeneratedFunction {
        ident: quote::format_ident!("prepare_{name}"),
        helper: "prepare helper",
    };
    match query.annotation {
        Annotation::One => vec![
            prepare,
            GeneratedFunction {
                ident: name.clone(),
                helper: "query function",
            },
            GeneratedFunction {
                ident: quote::format_ident!("{name}_with"),
                helper: "with helper",
            },
            GeneratedFunction {
                ident: quote::format_ident!("{name}_opt"),
                helper: "optional query helper",
            },
            GeneratedFunction {
                ident: quote::format_ident!("{name}_opt_with"),
                helper: "optional with helper",
            },
        ],
        Annotation::Many => {
            let suffix = backend.many_iterator_suffix();
            let (iterator, iterator_with) = backend.many_iterator_helpers();
            vec![
                prepare,
                GeneratedFunction {
                    ident: name.clone(),
                    helper: "query function",
                },
                GeneratedFunction {
                    ident: quote::format_ident!("{name}_with"),
                    helper: "with helper",
                },
                GeneratedFunction {
                    ident: quote::format_ident!("{name}_{suffix}"),
                    helper: iterator,
                },
                GeneratedFunction {
                    ident: quote::format_ident!("{name}_{suffix}_with"),
                    helper: iterator_with,
                },
            ]
        }
        Annotation::Exec | Annotation::ExecRows | Annotation::ExecResult => vec![
            prepare,
            GeneratedFunction {
                ident: name.clone(),
                helper: "query function",
            },
            GeneratedFunction {
                ident: quote::format_ident!("{name}_with"),
                helper: "with helper",
            },
        ],
        _ => unreachable!("unsupported annotation was filtered"),
    }
}

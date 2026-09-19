use crate::{
    db_crates::{
        DbCrate, Postgres,
        params_common::{self, GeneratedItem},
    },
    query::{Annotation, Query},
};

pub(super) fn generated_functions(backend: Postgres, query: &Query) -> Vec<GeneratedItem> {
    if !DbCrate::Postgres(backend).supports(query.annotation) {
        return Vec::new();
    }

    let name = params_common::query_function_ident(query);
    if query.dynfilter().is_some() {
        let helper = GeneratedItem {
            ident: quote::format_ident!("{name}_query"),
            helper: "dynamic query helper",
        };
        return match query.annotation {
            Annotation::One => vec![
                helper,
                GeneratedItem {
                    ident: name.clone(),
                    helper: "query function",
                },
                GeneratedItem {
                    ident: quote::format_ident!("{name}_opt"),
                    helper: "optional query helper",
                },
            ],
            Annotation::Many => {
                let suffix = backend.many_iterator_suffix();
                vec![
                    helper,
                    GeneratedItem {
                        ident: name.clone(),
                        helper: "query function",
                    },
                    GeneratedItem {
                        ident: quote::format_ident!("{name}_{suffix}"),
                        helper: backend.many_iterator_helpers().0,
                    },
                ]
            }
            Annotation::Exec | Annotation::ExecRows | Annotation::ExecResult => {
                vec![
                    helper,
                    GeneratedItem {
                        ident: name,
                        helper: "query function",
                    },
                ]
            }
            _ => unreachable!("unsupported annotation was filtered"),
        };
    }

    let prepare = GeneratedItem {
        ident: quote::format_ident!("prepare_{name}"),
        helper: "prepare helper",
    };
    match query.annotation {
        Annotation::One => vec![
            prepare,
            GeneratedItem {
                ident: name.clone(),
                helper: "query function",
            },
            GeneratedItem {
                ident: quote::format_ident!("{name}_with"),
                helper: "with helper",
            },
            GeneratedItem {
                ident: quote::format_ident!("{name}_opt"),
                helper: "optional query helper",
            },
            GeneratedItem {
                ident: quote::format_ident!("{name}_opt_with"),
                helper: "optional with helper",
            },
        ],
        Annotation::Many => {
            let suffix = backend.many_iterator_suffix();
            let (iterator, iterator_with) = backend.many_iterator_helpers();
            vec![
                prepare,
                GeneratedItem {
                    ident: name.clone(),
                    helper: "query function",
                },
                GeneratedItem {
                    ident: quote::format_ident!("{name}_with"),
                    helper: "with helper",
                },
                GeneratedItem {
                    ident: quote::format_ident!("{name}_{suffix}"),
                    helper: iterator,
                },
                GeneratedItem {
                    ident: quote::format_ident!("{name}_{suffix}_with"),
                    helper: iterator_with,
                },
            ]
        }
        Annotation::Exec | Annotation::ExecRows | Annotation::ExecResult => vec![
            prepare,
            GeneratedItem {
                ident: name.clone(),
                helper: "query function",
            },
            GeneratedItem {
                ident: quote::format_ident!("{name}_with"),
                helper: "with helper",
            },
        ],
        _ => unreachable!("unsupported annotation was filtered"),
    }
}

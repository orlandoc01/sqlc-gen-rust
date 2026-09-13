use crate::{
    db_crates::{
        DbCrate, Postgres,
        params_common::{self, GeneratedFunction},
    },
    query::{Annotation, Query},
};

struct FunctionNames {
    name: syn::Ident,
    opt: syn::Ident,
    with: syn::Ident,
    opt_with: syn::Ident,
    iterator: syn::Ident,
    iterator_with: syn::Ident,
}

fn function_names(backend: Postgres, query: &Query) -> FunctionNames {
    let name = params_common::query_function_ident(query);
    let suffix = backend.many_iterator_suffix();
    FunctionNames {
        opt: quote::format_ident!("{name}_opt"),
        with: quote::format_ident!("{name}_with"),
        opt_with: quote::format_ident!("{name}_opt_with"),
        iterator: quote::format_ident!("{name}_{suffix}"),
        iterator_with: quote::format_ident!("{name}_{suffix}_with"),
        name,
    }
}

pub(super) fn generated_functions(backend: Postgres, query: &Query) -> Vec<GeneratedFunction> {
    if !DbCrate::Postgres(backend).supports(query.annotation) {
        return Vec::new();
    }

    let names = function_names(backend, query);
    if query.dynfilter().is_some() {
        let helper = GeneratedFunction {
            ident: quote::format_ident!("{}_query", names.name),
            helper: "dynamic query helper",
        };
        return match query.annotation {
            Annotation::One => vec![
                helper,
                GeneratedFunction {
                    ident: names.name,
                    helper: "query function",
                },
                GeneratedFunction {
                    ident: names.opt,
                    helper: "optional query helper",
                },
            ],
            Annotation::Many => {
                vec![
                    helper,
                    GeneratedFunction {
                        ident: names.name,
                        helper: "query function",
                    },
                    GeneratedFunction {
                        ident: names.iterator,
                        helper: backend.many_iterator_helpers().0,
                    },
                ]
            }
            Annotation::Exec | Annotation::ExecRows | Annotation::ExecResult => {
                vec![
                    helper,
                    GeneratedFunction {
                        ident: names.name,
                        helper: "query function",
                    },
                ]
            }
            _ => unreachable!("unsupported annotation was filtered"),
        };
    }

    let prepare = GeneratedFunction {
        ident: quote::format_ident!("prepare_{}", names.name),
        helper: "prepare helper",
    };
    match query.annotation {
        Annotation::One => vec![
            prepare,
            GeneratedFunction {
                ident: names.name.clone(),
                helper: "query function",
            },
            GeneratedFunction {
                ident: names.with,
                helper: "with helper",
            },
            GeneratedFunction {
                ident: names.opt,
                helper: "optional query helper",
            },
            GeneratedFunction {
                ident: names.opt_with,
                helper: "optional with helper",
            },
        ],
        Annotation::Many => {
            let (iterator, iterator_with) = backend.many_iterator_helpers();
            vec![
                prepare,
                GeneratedFunction {
                    ident: names.name.clone(),
                    helper: "query function",
                },
                GeneratedFunction {
                    ident: names.with,
                    helper: "with helper",
                },
                GeneratedFunction {
                    ident: names.iterator,
                    helper: iterator,
                },
                GeneratedFunction {
                    ident: names.iterator_with,
                    helper: iterator_with,
                },
            ]
        }
        Annotation::Exec | Annotation::ExecRows | Annotation::ExecResult => vec![
            prepare,
            GeneratedFunction {
                ident: names.name.clone(),
                helper: "query function",
            },
            GeneratedFunction {
                ident: names.with,
                helper: "with helper",
            },
        ],
        _ => unreachable!("unsupported annotation was filtered"),
    }
}

use super::{
    params_common::{self, ParameterAccess, ParamsGenerator, QueryParts},
    sqlx::Sqlx,
};
use crate::query::{Annotation, Query, ReturningRows};

pub(crate) fn generate_queries(
    sqlx: &Sqlx,
    rows: &[ReturningRows],
    queries: &[Query],
    query_parameter_limit: usize,
) -> proc_macro2::TokenStream {
    params_common::generate_queries(sqlx, rows, queries, query_parameter_limit)
}

impl ParamsGenerator for Sqlx {
    fn placeholders(&self) -> proc_macro2::TokenStream {
        match self {
            Self::MySql => quote::quote! {dynfilter::Placeholders::Question},
            Self::Postgres => quote::quote! {dynfilter::Placeholders::Numbered},
            Self::Sqlite => quote::quote! {dynfilter::Placeholders::NumberedSqlite},
        }
    }

    fn returning_row(&self, row: &ReturningRows) -> proc_macro2::TokenStream {
        self.returning_ordinal_row(row)
    }

    fn query_functions(
        &self,
        query: &Query,
        row: &ReturningRows,
        parts: &QueryParts,
    ) -> proc_macro2::TokenStream {
        Function::new(self, query, row, parts).generate()
    }
}

/// Generated identifiers chosen so they never collide with direct SQL parameter names.
struct Function<'a> {
    sqlx: &'a Sqlx,
    query: &'a Query,
    row: &'a ReturningRows,
    parts: &'a QueryParts,
    name: syn::Ident,
    executor: syn::Ident,
    q: syn::Ident,
}

impl<'a> Function<'a> {
    fn new(
        sqlx: &'a Sqlx,
        query: &'a Query,
        row: &'a ReturningRows,
        parts: &'a QueryParts,
    ) -> Self {
        Self {
            sqlx,
            query,
            row,
            parts,
            name: params_common::query_function_ident(query),
            executor: parts.local("executor"),
            q: parts.local("q"),
        }
    }

    fn generate(&self) -> proc_macro2::TokenStream {
        let Self {
            name, executor, q, ..
        } = self;
        let database = self.sqlx.database_ident();
        let arguments = &self.parts.arguments;
        match self.query.annotation {
            Annotation::One => {
                let row = self.row.struct_ident();
                let setup = self.setup(Some(&row));
                let opt_name = quote::format_ident!("{name}_opt");
                quote::quote! {
                    pub async fn #name<'e>(
                        #executor: impl sqlx::Executor<'e, Database = #database>
                        #arguments
                    ) -> Result<#row, sqlx::Error> {
                        #setup
                        #q.fetch_one(#executor).await
                    }

                    pub async fn #opt_name<'e>(
                        #executor: impl sqlx::Executor<'e, Database = #database>
                        #arguments
                    ) -> Result<Option<#row>, sqlx::Error> {
                        #setup
                        #q.fetch_optional(#executor).await
                    }
                }
            }
            Annotation::Many => {
                let row = self.row.struct_ident();
                let setup = self.setup(Some(&row));
                quote::quote! {
                    pub async fn #name<'e>(
                        #executor: impl sqlx::Executor<'e, Database = #database>
                        #arguments
                    ) -> Result<Vec<#row>, sqlx::Error> {
                        #setup
                        #q.fetch_all(#executor).await
                    }
                }
            }
            Annotation::Exec => self.execute(quote::quote! {()}, quote::quote! {.map(|_| ())}),
            Annotation::ExecRows => self.execute(
                quote::quote! {u64},
                quote::quote! {.map(|result| result.rows_affected())},
            ),
            Annotation::ExecResult => self.execute(
                quote::quote! {<#database as sqlx::Database>::QueryResult},
                proc_macro2::TokenStream::new(),
            ),
            Annotation::ExecLastId => match self.sqlx {
                Sqlx::Sqlite => self.execute(
                    quote::quote! {i64},
                    quote::quote! {.map(|result| result.last_insert_rowid())},
                ),
                Sqlx::MySql => self.execute(
                    quote::quote! {u64},
                    quote::quote! {.map(|result| result.last_insert_id())},
                ),
                Sqlx::Postgres => proc_macro2::TokenStream::new(),
            },
            Annotation::BatchExec
            | Annotation::BatchMany
            | Annotation::BatchOne
            | Annotation::CopyFrom => proc_macro2::TokenStream::new(),
        }
    }

    fn execute(
        &self,
        return_type: proc_macro2::TokenStream,
        map: proc_macro2::TokenStream,
    ) -> proc_macro2::TokenStream {
        let Self {
            name, executor, q, ..
        } = self;
        let database = self.sqlx.database_ident();
        let arguments = &self.parts.arguments;
        let setup = self.setup(None);
        quote::quote! {
            pub async fn #name<'e>(
                #executor: impl sqlx::Executor<'e, Database = #database>
                #arguments
            ) -> Result<#return_type, sqlx::Error> {
                #setup
                #q.execute(#executor).await #map
            }
        }
    }

    fn setup(&self, row: Option<&syn::Ident>) -> proc_macro2::TokenStream {
        if self.query.dynfilter().is_some() {
            return self.dynamic_setup(row);
        }
        let q = &self.q;
        let constant = &self.parts.constant;
        let bind = match self.parts.access {
            ParameterAccess::Direct => {
                self.sqlx
                    .query_bind(self.query, q.clone(), |name| quote::quote! {#name})
            }
            ParameterAccess::Struct => {
                self.sqlx
                    .query_bind(self.query, q.clone(), |name| quote::quote! {params.#name})
            }
        };
        let query = match row {
            Some(row) => quote::quote! {sqlx::query_as::<_, #row>(#constant)},
            None => quote::quote! {sqlx::query(#constant)},
        };
        quote::quote! {
            let #q = #query;
            #bind
        }
    }

    fn dynamic_setup(&self, row: Option<&syn::Ident>) -> proc_macro2::TokenStream {
        let q = &self.q;
        let dynamic = quote::format_ident!("{}_DYN", self.parts.constant);
        let args = params_common::dynamic_args(self.query);
        let binds = params_common::dynamic_binds(self.query)
            .into_iter()
            .map(|bind| {
                let pattern = bind.pattern;
                let name = &bind.field.name;
                if let Some(element) = bind.slice_element {
                    return quote::quote! {
                        #pattern => {
                            let elem = #element;
                            #q.bind(elem)
                        }
                    };
                }
                let scalar = bind.field.scalar_type();
                let by_value = scalar.copy_cheap() || scalar.need_params_struct_lifetime();
                let value = match (bind.conditional, by_value) {
                    (true, true) => quote::quote! {params.#name.unwrap()},
                    (true, false) => quote::quote! {params.#name.as_ref().unwrap()},
                    (false, true) => quote::quote! {params.#name},
                    (false, false) => quote::quote! {&params.#name},
                };
                quote::quote! {#pattern => #q.bind(#value),}
            });
        let query = match row {
            Some(row) => quote::quote! {sqlx::query_as::<_, #row>(&sql)},
            None => quote::quote! {sqlx::query(&sql)},
        };

        quote::quote! {
            let args = [#(#args,)*];
            let (sql, binds) = #dynamic.build(&args);
            let mut #q = #query;
            for bind in binds {
                #q = match bind {
                    #(#binds)*
                    _ => unreachable!("dynfilter bind plan referenced an unknown argument"),
                };
            }
            let #q = #q.persistent(false);
        }
    }
}

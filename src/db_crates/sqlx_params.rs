use super::{
    DbCrate,
    params_common::{self, GeneratedItem, ParamsGenerator, PreparedParts, QueryParts, Warmup},
    sqlx::Sqlx,
};
use crate::dynfilter_runtime::{Dialect, Placeholders};
use crate::query::{Annotation, Query, QueryError, ReturningRows};

mod prepared;

pub(crate) use prepared::bind_classes;

impl ParamsGenerator for Sqlx {
    fn placeholders(&self) -> Placeholders {
        match self {
            Self::MySql => Placeholders::Question,
            Self::Postgres => Placeholders::Numbered,
            Self::Sqlite => Placeholders::NumberedSqlite,
        }
    }

    fn dialect(&self) -> Dialect {
        match self {
            Self::MySql => Dialect::MySql,
            Self::Postgres => Dialect::Postgres,
            Self::Sqlite => Dialect::Sqlite,
        }
    }

    fn returning_row(&self, row: &ReturningRows) -> proc_macro2::TokenStream {
        let struct_tokens = super::make_return_row(row);
        let ident = row.struct_ident();
        let row_type = self.row_type();
        let fields = super::row_field_initializers(row, |index| {
            quote::quote! { sqlx::Row::try_get(row, #index)? }
        });
        quote::quote! {
            #struct_tokens
            impl<'r> sqlx::FromRow<'r, #row_type> for #ident {
                fn from_row(row: &'r #row_type) -> Result<Self, sqlx::Error> {
                    Ok(Self { #(#fields,)* })
                }
            }
        }
    }

    fn generated_functions(&self, query: &Query) -> Vec<GeneratedItem> {
        params_common::simple_generated_functions(query, |annotation| {
            DbCrate::Sqlx(*self).supports(annotation)
        })
    }

    fn query_functions(
        &self,
        query: &Query,
        row: &ReturningRows,
        parts: &QueryParts<'_>,
    ) -> proc_macro2::TokenStream {
        Function::new(self, query, row, parts).generate()
    }

    fn caches_statements(&self) -> bool {
        true
    }

    fn validate_prepared(
        &self,
        queries: &[Query],
        prepared: &crate::dynfilter::prepared::Prepared<'_>,
    ) -> Result<(), QueryError> {
        match self {
            Self::Postgres => prepared::validate_bind_types(queries, prepared),
            Self::MySql | Self::Sqlite => Ok(()),
        }
    }

    fn warmup(&self) -> Warmup {
        let connection = self.connection_ident();
        Warmup {
            param: "conn",
            client: quote::quote! {&mut #connection},
            result: quote::quote! {Result<(), sqlx::Error>},
            asynchronous: true,
        }
    }

    fn warmup_body(&self, query: &Query, parts: &PreparedParts<'_>) -> proc_macro2::TokenStream {
        prepared::warmup_body(self, query, parts)
    }
}

/// Generated identifiers chosen so they never collide with direct SQL parameter names.
struct Function<'a> {
    sqlx: &'a Sqlx,
    query: &'a Query,
    row: &'a ReturningRows,
    parts: &'a QueryParts<'a>,
    name: syn::Ident,
    executor: syn::Ident,
    q: syn::Ident,
}

impl<'a> Function<'a> {
    fn new(
        sqlx: &'a Sqlx,
        query: &'a Query,
        row: &'a ReturningRows,
        parts: &'a QueryParts<'a>,
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
        let access = self.parts.access;
        let bind = self
            .sqlx
            .query_bind(self.query, q.clone(), |name| access.field(name));
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
        let dynamic_setup = params_common::dynamic_plan_setup(self.query, self.parts);
        let unknown_bind_arm = params_common::unknown_bind_arm();
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
            })
            .collect::<Vec<_>>();
        let sql = self.parts.sql_ref();
        let query = match row {
            Some(row) => quote::quote! {sqlx::query_as::<_, #row>(#sql)},
            None => quote::quote! {sqlx::query(#sql)},
        };

        // A cache-eligible query keeps sqlx's per-connection statement cache; every other
        // dynamic query still bypasses it, since its texts are unbounded or not enumerated.
        let persistent = self
            .parts
            .prepared
            .is_none()
            .then(|| quote::quote! { let #q = #q.persistent(false); });
        if binds.is_empty() {
            return quote::quote! {
                #dynamic_setup
                debug_assert!(binds.is_empty(), "dynfilter bind plan referenced an unknown argument");
                let #q = #query;
                #persistent
            };
        }
        quote::quote! {
            #dynamic_setup
            let mut #q = #query;
            for bind in binds.iter().copied() {
                #q = match bind {
                    #(#binds)*
                    #unknown_bind_arm
                };
            }
            #persistent
        }
    }
}

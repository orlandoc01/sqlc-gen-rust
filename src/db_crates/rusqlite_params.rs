use crate::dynfilter_runtime::{Dialect, Placeholders};
use crate::query::{Annotation, Query, ReturningRows};

use super::{
    DbCrate,
    params_common::{self, GeneratedItem, ParamsGenerator, PreparedParts, QueryParts, Warmup},
    rusqlite::Rusqlite,
};

impl ParamsGenerator for Rusqlite {
    fn placeholders(&self) -> Placeholders {
        Placeholders::NumberedSqlite
    }

    fn dialect(&self) -> Dialect {
        Dialect::Sqlite
    }

    fn returning_row(&self, row: &ReturningRows) -> proc_macro2::TokenStream {
        super::ordinal_from_row(
            row,
            quote::quote! {rusqlite::Row<'_>},
            quote::quote! {rusqlite::Result<Self>},
            |index| quote::quote! {row.get(#index)?},
        )
    }

    fn generated_functions(&self, query: &Query) -> Vec<GeneratedItem> {
        params_common::simple_generated_functions(query, |annotation| {
            DbCrate::Rusqlite.supports(annotation)
        })
    }

    fn query_functions(
        &self,
        query: &Query,
        row: &ReturningRows,
        parts: &QueryParts<'_>,
    ) -> proc_macro2::TokenStream {
        Function::new(query, row, parts).generate()
    }

    fn caches_statements(&self) -> bool {
        true
    }

    fn warmup(&self) -> Warmup {
        Warmup {
            param: "client",
            client: quote::quote! {&impl RusqliteClient},
            result: quote::quote! {rusqlite::Result<()>},
            asynchronous: false,
        }
    }

    fn warmup_body(&self, _query: &Query, parts: &PreparedParts<'_>) -> proc_macro2::TokenStream {
        self.warmup().prepare_each(parts, |client| {
            quote::quote! { #client.connection().prepare_cached(sql) }
        })
    }
}

/// Generated identifiers chosen so they never collide with direct SQL parameter names.
struct Function<'a> {
    query: &'a Query,
    row: &'a ReturningRows,
    parts: &'a QueryParts<'a>,
    name: syn::Ident,
    client: syn::Ident,
    statement: syn::Ident,
    params: syn::Ident,
}

impl<'a> Function<'a> {
    fn new(query: &'a Query, row: &'a ReturningRows, parts: &'a QueryParts<'a>) -> Self {
        Self {
            query,
            row,
            parts,
            name: params_common::query_function_ident(query),
            client: parts.local("client"),
            statement: parts.local("statement"),
            params: parts.local("params"),
        }
    }

    fn generate(&self) -> proc_macro2::TokenStream {
        let Self {
            name,
            client,
            statement,
            params,
            ..
        } = self;
        let arguments = &self.parts.arguments;
        let setup = self.setup();
        match self.query.annotation {
            Annotation::One => {
                let row = self.row.struct_ident();
                let opt_name = quote::format_ident!("{name}_opt");
                quote::quote! {
                    pub fn #name(#client: &impl RusqliteClient #arguments) -> rusqlite::Result<#row> {
                        #setup
                        #statement.query_row(#params, #row::from_row)
                    }

                    pub fn #opt_name(#client: &impl RusqliteClient #arguments) -> rusqlite::Result<Option<#row>> {
                        use rusqlite::OptionalExtension as _;

                        #setup
                        #statement.query_row(#params, #row::from_row).optional()
                    }
                }
            }
            Annotation::Many => {
                let row = self.row.struct_ident();
                quote::quote! {
                    pub fn #name(#client: &impl RusqliteClient #arguments) -> rusqlite::Result<Vec<#row>> {
                        #setup
                        #statement.query_map(#params, #row::from_row)?.collect()
                    }
                }
            }
            Annotation::Exec => self.execute(quote::quote! {()}, quote::quote! {()}),
            Annotation::ExecRows => self.execute(
                quote::quote! {u64},
                quote::quote! {#client.connection().changes()},
            ),
            Annotation::ExecLastId => self.execute(
                quote::quote! {i64},
                quote::quote! {#client.connection().last_insert_rowid()},
            ),
            Annotation::ExecResult
            | Annotation::BatchExec
            | Annotation::BatchMany
            | Annotation::BatchOne
            | Annotation::CopyFrom => proc_macro2::TokenStream::new(),
        }
    }

    /// Steps the statement to completion instead of `execute`, so a DML statement with
    /// `RETURNING` reports success after its write rather than `ExecuteReturnedResults`.
    fn execute(
        &self,
        return_type: proc_macro2::TokenStream,
        result: proc_macro2::TokenStream,
    ) -> proc_macro2::TokenStream {
        let Self {
            name,
            client,
            statement,
            params,
            ..
        } = self;
        let arguments = &self.parts.arguments;
        let setup = self.setup();
        quote::quote! {
            pub fn #name(#client: &impl RusqliteClient #arguments) -> rusqlite::Result<#return_type> {
                #setup
                let mut rows = #statement.query(#params)?;
                while rows.next()?.is_some() {}
                Ok(#result)
            }
        }
    }

    fn setup(&self) -> proc_macro2::TokenStream {
        if self.query.dynfilter().is_some() {
            return self.dynamic_setup();
        }

        let Self {
            client,
            statement,
            params,
            ..
        } = self;
        let constant = &self.parts.constant;
        let values = self.query.fields.iter().map(|field| {
            let name = &field.name;
            self.parts.access.field(name)
        });
        quote::quote! {
            let #params = rusqlite::params![#(#values),*];
            let mut #statement = #client.connection().prepare_cached(#constant)?;
        }
    }

    fn dynamic_setup(&self) -> proc_macro2::TokenStream {
        let Self {
            client,
            statement,
            params,
            ..
        } = self;
        let dynamic_setup = params_common::dynamic_plan_setup(self.query, self.parts);
        let unknown_bind_arm = params_common::unknown_bind_arm();
        let binds = params_common::dynamic_binds(self.query)
            .into_iter()
            .map(|bind| {
                let pattern = bind.pattern;
                let name = &bind.field.name;
                let value = match (bind.slice_element, bind.conditional) {
                    (Some(element), _) => element,
                    (None, true) => quote::quote! {params.#name.as_ref().unwrap()},
                    (None, false) => quote::quote! {&params.#name},
                };
                quote::quote! {#pattern => #value,}
            })
            .collect::<Vec<_>>();
        let value = if binds.is_empty() {
            quote::quote! {|_| -> &dyn rusqlite::ToSql {
                unreachable!("dynfilter bind plan referenced an unknown argument")
            }}
        } else {
            quote::quote! {|bind| -> &dyn rusqlite::ToSql {
                match bind {
                    #(#binds)*
                    #unknown_bind_arm
                }
            }}
        };
        let sql = self.parts.sql_ref();
        let prepare = if self.parts.prepared.is_some() {
            quote::quote! { prepare_cached(#sql) }
        } else {
            quote::quote! { prepare(#sql) }
        };
        quote::quote! {
            #dynamic_setup
            let values = binds.iter().copied().map(#value);
            let mut #statement = #client.connection().#prepare?;
            let #params = rusqlite::params_from_iter(values);
        }
    }
}

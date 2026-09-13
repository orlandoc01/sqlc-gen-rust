use crate::db_crates::postgres::ClientSignature;

use super::{Function, StaticParts, typed};

impl Function<'_> {
    pub(super) fn one_functions(&self) -> proc_macro2::TokenStream {
        let typed_parameters = self.typed_parameters();
        let parts = self.static_parts();
        let name = &self.name;
        let opt = quote::format_ident!("{name}_opt");
        let with = quote::format_ident!("{name}_with");
        let opt_with = quote::format_ident!("{name}_opt_with");
        let row = self.row.struct_ident();
        let client = &self.client;
        let constant = &self.parts.constant;
        let await_token = &self.paths.await_token;
        let statement = &parts.statement;
        let values_ident = &parts.values_ident;
        let forwarded = &parts.forwarded;
        let one_body = match &typed_parameters {
            Some(parameters) => {
                let values = typed::values(self, values_ident, parameters);
                quote::quote! {
                    #values
                    let row = #client.query_typed_one(#constant, #values_ident) #await_token?;
                    #row::from_row(&row)
                }
            }
            None => quote::quote! { self::#with(#client, #constant #forwarded) #await_token },
        };
        let optional_body = match &typed_parameters {
            Some(parameters) => {
                let values = typed::values(self, values_ident, parameters);
                quote::quote! {
                    #values
                    #client.query_typed_opt(#constant, #values_ident) #await_token?.map(|row| #row::from_row(&row)).transpose()
                }
            }
            None => quote::quote! { self::#opt_with(#client, #constant #forwarded) #await_token },
        };
        let one = self.plain_fn(
            &self.paths.plain,
            name,
            &quote::quote! {#row},
            typed_parameters.is_some(),
            one_body,
        );
        let optional = self.plain_fn(
            &self.paths.plain,
            &opt,
            &quote::quote! {Option<#row>},
            typed_parameters.is_some(),
            optional_body,
        );
        let one_with = self.with_fn(
            &self.paths.plain,
            &with,
            &quote::quote! {#row},
            &parts,
            quote::quote! {
                let row = #client.query_one(#statement, #values_ident) #await_token?;
                #row::from_row(&row)
            },
        );
        let optional_with = self.with_fn(
            &self.paths.plain,
            &opt_with,
            &quote::quote! {Option<#row>},
            &parts,
            quote::quote! {
                #client.query_opt(#statement, #values_ident) #await_token?.map(|row| #row::from_row(&row)).transpose()
            },
        );
        let prepare = self.prepare_function();
        quote::quote! { #prepare #one #one_with #optional #optional_with }
    }

    pub(super) fn many_functions(&self) -> proc_macro2::TokenStream {
        let typed_parameters = self.typed_parameters();
        let parts = self.static_parts();
        let name = &self.name;
        let with = quote::format_ident!("{name}_with");
        let suffix = self.backend.many_iterator_suffix();
        let iterator = quote::format_ident!("{name}_{suffix}");
        let iterator_with = quote::format_ident!("{name}_{suffix}_with");
        let row = self.row.struct_ident();
        let client = &self.client;
        let constant = &self.parts.constant;
        let await_token = &self.paths.await_token;
        let statement = &parts.statement;
        let values_ident = &parts.values_ident;
        let forwarded = &parts.forwarded;
        let many_body = match &typed_parameters {
            Some(parameters) => {
                let values = typed::values(self, values_ident, parameters);
                quote::quote! {
                    #values
                    let rows = #client.query_typed(#constant, #values_ident) #await_token?;
                    rows.iter().map(#row::from_row).collect()
                }
            }
            None => quote::quote! { self::#with(#client, #constant #forwarded) #await_token },
        };
        let iterator_body = match &typed_parameters {
            Some(parameters) => {
                let values = typed::values(self, values_ident, parameters);
                quote::quote! {
                    #values
                    #client.query_typed_raw(#constant, #values_ident.iter().map(|(value, typ)| (*value, typ.clone()))) #await_token
                }
            }
            None => {
                quote::quote! { self::#iterator_with(#client, #constant #forwarded) #await_token }
            }
        };
        let many = self.plain_fn(
            &self.paths.plain,
            name,
            &quote::quote! {Vec<#row>},
            typed_parameters.is_some(),
            many_body,
        );
        let iterator_fn = self.plain_fn(
            &self.paths.iterator,
            &iterator,
            &self.paths.row_iter,
            typed_parameters.is_some(),
            iterator_body,
        );
        let many_with = self.with_fn(
            &self.paths.plain,
            &with,
            &quote::quote! {Vec<#row>},
            &parts,
            quote::quote! {
                let rows = #client.query(#statement, #values_ident) #await_token?;
                rows.iter().map(#row::from_row).collect()
            },
        );
        let iterator_with_fn = self.with_fn(
            &self.paths.iterator,
            &iterator_with,
            &self.paths.row_iter,
            &parts,
            quote::quote! {
                #client.query_raw(#statement, #values_ident.iter().copied()) #await_token
            },
        );
        let prepare = self.prepare_function();
        quote::quote! { #prepare #many #many_with #iterator_fn #iterator_with_fn }
    }

    pub(super) fn execute_functions(
        &self,
        result: proc_macro2::TokenStream,
        map: proc_macro2::TokenStream,
    ) -> proc_macro2::TokenStream {
        let typed_parameters = self.typed_parameters();
        let parts = self.static_parts();
        let name = &self.name;
        let with = quote::format_ident!("{name}_with");
        let client = &self.client;
        let statement = &parts.statement;
        let values_ident = &parts.values_ident;
        let await_token = &self.paths.await_token;
        let constant = &self.parts.constant;
        let forwarded = &parts.forwarded;
        let typed_body = typed_parameters.as_ref().map(|parameters| {
            let values = typed::values(self, values_ident, parameters);
            let raw = typed::execute_raw(
                self,
                quote::quote! {#constant},
                quote::quote! {#values_ident},
                matches!(
                    self.query.annotation,
                    crate::query::Annotation::ExecRows | crate::query::Annotation::ExecResult
                ),
            );
            quote::quote! { #values #raw }
        });
        let execute = self.plain_fn(
            &self.paths.plain,
            name,
            &result,
            typed_parameters.is_some(),
            typed_body.unwrap_or_else(|| {
                quote::quote! { self::#with(#client, #constant #forwarded) #await_token }
            }),
        );
        let execute_with = self.with_fn(
            &self.paths.plain,
            &with,
            &result,
            &parts,
            quote::quote! { #client.execute(#statement, #values_ident) #await_token #map },
        );
        let prepare = self.prepare_function();
        quote::quote! { #prepare #execute #execute_with }
    }

    pub(super) fn plain_fn(
        &self,
        signature: &ClientSignature,
        name: &syn::Ident,
        return_type: &proc_macro2::TokenStream,
        typed: bool,
        body: proc_macro2::TokenStream,
    ) -> proc_macro2::TokenStream {
        let client = &self.client;
        let arguments = &self.parts.arguments;
        let generic_client = &self.paths.client;
        let error = &self.paths.error;
        let async_token = &self.paths.async_token;
        let lifetime = &signature.lifetime;
        let client_ref = &signature.client_ref;
        let client_type = if typed {
            typed::client_type(self, signature)
        } else {
            quote::quote! {#client_ref impl #generic_client}
        };
        quote::quote! {
            pub #async_token fn #name #lifetime(#client: #client_type #arguments) -> Result<#return_type, #error> {
                #body
            }
        }
    }

    pub(super) fn with_fn(
        &self,
        signature: &ClientSignature,
        name: &syn::Ident,
        return_type: &proc_macro2::TokenStream,
        parts: &StaticParts,
        body: proc_macro2::TokenStream,
    ) -> proc_macro2::TokenStream {
        let client = &self.client;
        let statement = &parts.statement;
        let arguments = &self.parts.arguments;
        let values = &parts.values;
        let generic_client = &self.paths.client;
        let to_statement = &self.paths.to_statement;
        let error = &self.paths.error;
        let async_token = &self.paths.async_token;
        let lifetime = &signature.lifetime;
        let client_ref = &signature.client_ref;
        quote::quote! {
            pub #async_token fn #name #lifetime(#client: #client_ref impl #generic_client, #statement: &(impl #to_statement + ?::std::marker::Sized + ::std::marker::Sync + ::std::marker::Send) #arguments) -> Result<#return_type, #error> {
                #values
                #body
            }
        }
    }

    pub(super) fn prepare_function(&self) -> proc_macro2::TokenStream {
        let prepare = quote::format_ident!("prepare_{}", self.name);
        let prepare_statement = self
            .backend
            .prepare_statement(&self.client, &self.parts.constant);
        let client = &self.client;
        let generic_client = &self.paths.client;
        let statement = &self.paths.statement;
        let error = &self.paths.error;
        let async_token = &self.paths.async_token;
        let lifetime = &self.paths.plain.lifetime;
        let client_ref = &self.paths.plain.client_ref;
        quote::quote! {
            pub #async_token fn #prepare #lifetime(#client: #client_ref impl #generic_client) -> Result<#statement, #error> {
                #prepare_statement
            }
        }
    }
}

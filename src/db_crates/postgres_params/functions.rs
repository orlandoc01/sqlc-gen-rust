use crate::db_crates::postgres::ClientSignature;

use super::{Function, StaticParts};

impl Function<'_> {
    pub(super) fn one_functions(&self) -> proc_macro2::TokenStream {
        let parts = self.static_parts();
        let name = &self.name;
        let opt = quote::format_ident!("{name}_opt");
        let with = quote::format_ident!("{name}_with");
        let opt_with = quote::format_ident!("{name}_opt_with");
        let row = self.row.struct_ident();
        let client = &self.client;
        let statement = &parts.statement;
        let values_ident = &parts.values_ident;
        let await_token = &self.paths.await_token;
        let row_type = quote::quote! {#row};
        let optional_row_type = quote::quote! {Option<#row>};
        let one = self.plain_fn(&self.paths.plain, name, &row_type, &with, &parts.forwarded);
        let one_with = self.with_fn(
            &self.paths.plain,
            &with,
            &row_type,
            &parts,
            quote::quote! {
                let row = #client.query_one(#statement, #values_ident) #await_token?;
                #row::from_row(&row)
            },
        );
        let optional = self.plain_fn(
            &self.paths.plain,
            &opt,
            &optional_row_type,
            &opt_with,
            &parts.forwarded,
        );
        let optional_with = self.with_fn(
            &self.paths.plain,
            &opt_with,
            &optional_row_type,
            &parts,
            quote::quote! {
                #client.query_opt(#statement, #values_ident) #await_token?.map(|row| #row::from_row(&row)).transpose()
            },
        );
        let prepare = self.prepare_function();
        quote::quote! {
            #prepare
            #one
            #one_with
            #optional
            #optional_with
        }
    }

    pub(super) fn many_functions(&self) -> proc_macro2::TokenStream {
        let parts = self.static_parts();
        let name = &self.name;
        let with = quote::format_ident!("{name}_with");
        let suffix = self.backend.many_iterator_suffix();
        let iterator = quote::format_ident!("{name}_{suffix}");
        let iterator_with = quote::format_ident!("{name}_{suffix}_with");
        let row = self.row.struct_ident();
        let client = &self.client;
        let statement = &parts.statement;
        let values_ident = &parts.values_ident;
        let await_token = &self.paths.await_token;
        let many_type = quote::quote! {Vec<#row>};
        let iterator_type = &self.paths.row_iter;
        let many = self.plain_fn(&self.paths.plain, name, &many_type, &with, &parts.forwarded);
        let many_with = self.with_fn(
            &self.paths.plain,
            &with,
            &many_type,
            &parts,
            quote::quote! {
                let rows = #client.query(#statement, #values_ident) #await_token?;
                rows.iter().map(#row::from_row).collect()
            },
        );
        let iterator_fn = self.plain_fn(
            &self.paths.iterator,
            &iterator,
            iterator_type,
            &iterator_with,
            &parts.forwarded,
        );
        let iterator_with_fn = self.with_fn(
            &self.paths.iterator,
            &iterator_with,
            iterator_type,
            &parts,
            quote::quote! {
                #client.query_raw(#statement, #values_ident.iter().copied()) #await_token
            },
        );
        let prepare = self.prepare_function();
        quote::quote! {
            #prepare
            #many
            #many_with
            #iterator_fn
            #iterator_with_fn
        }
    }

    pub(super) fn execute_functions(
        &self,
        result: proc_macro2::TokenStream,
        map: proc_macro2::TokenStream,
    ) -> proc_macro2::TokenStream {
        let parts = self.static_parts();
        let name = &self.name;
        let with = quote::format_ident!("{name}_with");
        let client = &self.client;
        let statement = &parts.statement;
        let values_ident = &parts.values_ident;
        let await_token = &self.paths.await_token;
        let execute = self.plain_fn(&self.paths.plain, name, &result, &with, &parts.forwarded);
        let execute_with = self.with_fn(
            &self.paths.plain,
            &with,
            &result,
            &parts,
            quote::quote! { #client.execute(#statement, #values_ident) #await_token #map },
        );
        let prepare = self.prepare_function();
        quote::quote! {
            #prepare
            #execute
            #execute_with
        }
    }

    fn plain_fn(
        &self,
        signature: &ClientSignature,
        name: &syn::Ident,
        return_type: &proc_macro2::TokenStream,
        with_name: &syn::Ident,
        forwarded: &proc_macro2::TokenStream,
    ) -> proc_macro2::TokenStream {
        let client = &self.client;
        let constant = &self.parts.constant;
        let arguments = &self.parts.arguments;
        let generic_client = &self.paths.client;
        let error = &self.paths.error;
        let async_token = &self.paths.async_token;
        let lifetime = &signature.lifetime;
        let client_ref = &signature.client_ref;
        let await_token = &self.paths.await_token;
        quote::quote! {
            pub #async_token fn #name #lifetime(#client: #client_ref impl #generic_client #arguments) -> Result<#return_type, #error> {
                self::#with_name(#client, #constant #forwarded) #await_token
            }
        }
    }

    fn with_fn(
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

    fn prepare_function(&self) -> proc_macro2::TokenStream {
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

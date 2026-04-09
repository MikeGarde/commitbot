use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, DeriveInput, Data, DataStruct, Fields};

#[proc_macro_derive(SensitiveFields, attributes(sensitive))]
pub fn derive_sensitive_fields(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = input.ident;

    let mut sensitive_fields: Vec<String> = Vec::new();

    if let Data::Struct(DataStruct { fields: Fields::Named(ref fields_named), .. }) = input.data {
        for field in fields_named.named.iter() {
            for attr in &field.attrs {
                if let Some(ident) = attr.path().get_ident() {
                    if ident == "sensitive" {
                        if let Some(ident_field) = &field.ident {
                            sensitive_fields.push(ident_field.to_string());
                        }
                    }
                }
            }
        }
    }

    // build an array of string literals
    let literals: Vec<proc_macro2::TokenStream> = sensitive_fields
        .iter()
        .map(|s| {
            let lit = syn::LitStr::new(s, proc_macro2::Span::call_site());
            quote! { #lit }
        })
        .collect();

    let expanded = quote! {
        impl #name {
            pub fn sensitive_field_names() -> &'static [&'static str] {
                &[#(#literals),*]
            }
        }
    };

    TokenStream::from(expanded)
}

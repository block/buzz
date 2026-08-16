//! Policy-enforcing instrumentation for logical datastore operations.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::{
    parenthesized, parse_macro_input, Error, Ident, ItemFn, LitStr, Result, ReturnType, Token, Type,
};

struct DatastoreArgs {
    name: LitStr,
    system: LitStr,
    fields: Option<TokenStream2>,
}

impl Parse for DatastoreArgs {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let mut name = None;
        let mut system = None;
        let mut fields = None;

        while !input.is_empty() {
            let key: Ident = input.parse()?;
            match key.to_string().as_str() {
                "name" => {
                    if name.is_some() {
                        return Err(Error::new(key.span(), "duplicate `name` argument"));
                    }
                    input.parse::<Token![=]>()?;
                    name = Some(input.parse()?);
                }
                "system" => {
                    if system.is_some() {
                        return Err(Error::new(key.span(), "duplicate `system` argument"));
                    }
                    input.parse::<Token![=]>()?;
                    system = Some(input.parse()?);
                }
                "fields" => {
                    if fields.is_some() {
                        return Err(Error::new(key.span(), "duplicate `fields` argument"));
                    }
                    let content;
                    parenthesized!(content in input);
                    fields = Some(content.parse()?);
                }
                _ => {
                    return Err(Error::new(
                        key.span(),
                        "expected `name`, `system`, or `fields`",
                    ))
                }
            }
            if !input.is_empty() {
                input.parse::<Token![,]>()?;
            }
        }

        Ok(Self {
            name: name.ok_or_else(|| Error::new(input.span(), "missing `name`"))?,
            system: system.ok_or_else(|| Error::new(input.span(), "missing `system`"))?,
            fields,
        })
    }
}

/// Instruments an async logical datastore operation according to Buzz policy.
///
/// PostgreSQL spans always omit function arguments, use the `buzz_datastore`
/// target, and expose only canonical semantic fields plus explicitly supplied
/// safe fields. An `Err` sets `otel.status_code` without inspecting the error.
#[proc_macro_attribute]
pub fn datastore_span(args: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args as DatastoreArgs);
    let mut function = parse_macro_input!(item as ItemFn);

    if function.sig.asyncness.is_none() {
        return Error::new_spanned(
            function.sig.fn_token,
            "`datastore_span` requires an async function",
        )
        .into_compile_error()
        .into();
    }
    if args.system.value() != "postgresql" {
        return Error::new_spanned(
            args.system,
            "unsupported datastore system; only `postgresql` is currently supported",
        )
        .into_compile_error()
        .into();
    }

    let name = args.name;
    let extra_fields = args.fields.map(|tokens| quote!(, #tokens));
    function.attrs.push(syn::parse_quote!(
        #[::tracing::instrument(
            target = "buzz_datastore",
            name = #name,
            skip_all,
            fields(
                otel.kind = "client",
                db.system.name = "postgresql",
                otel.status_code = ::tracing::field::Empty
                #extra_fields
            )
        )]
    ));

    let return_type: Type = match &function.sig.output {
        ReturnType::Type(_, ty) => *ty.clone(),
        ReturnType::Default => syn::parse_quote!(()),
    };
    let returns_result = match &return_type {
        Type::Path(path) => path
            .path
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "Result"),
        _ => false,
    };
    let original_body = function.block;
    let result = format_ident!("__buzz_datastore_result_7f3a9c");
    let record_error = returns_result.then(|| {
        quote! {
            if #result.is_err() {
                ::tracing::Span::current().record("otel.status_code", "ERROR");
            }
        }
    });
    function.block = Box::new(syn::parse_quote!({
        let #result: #return_type = (async #original_body).await;
        #record_error
        #result
    }));

    quote!(#function).into()
}

#[cfg(test)]
mod tests {
    use super::DatastoreArgs;

    #[test]
    fn parses_safe_tracing_fields() {
        let args: DatastoreArgs = syn::parse_str(
            r#"name = "audit", system = "postgresql", fields(action = %entry.action, from_seq = from_seq, limit = limit)"#,
        )
        .expect("valid arguments");
        assert!(args.fields.is_some());
    }

    #[test]
    fn rejects_duplicate_arguments() {
        for args in [
            r#"name = "one", name = "two", system = "postgresql""#,
            r#"name = "one", system = "postgresql", system = "postgresql""#,
            r#"name = "one", system = "postgresql", fields(a = 1), fields(b = 2)"#,
        ] {
            let error = match syn::parse_str::<DatastoreArgs>(args) {
                Ok(_) => panic!("duplicate accepted"),
                Err(error) => error,
            };
            assert!(error.to_string().starts_with("duplicate `"));
        }
    }
}

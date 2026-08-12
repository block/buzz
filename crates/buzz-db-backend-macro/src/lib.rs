//! Compile-time enforcement for explicit SQLite decisions on public `Db` methods.
//!
//! The impl-level attribute inspects only public associated methods. Private and
//! `pub(crate)` plumbing is intentionally outside this policy boundary.

use proc_macro::TokenStream;
use quote::quote;
use syn::spanned::Spanned;
use syn::{parse_macro_input, Attribute, ImplItem, ItemImpl, LitStr, Visibility};

#[proc_macro_attribute]
pub fn enforce_sqlite_backend_declarations(_args: TokenStream, input: TokenStream) -> TokenStream {
    let mut item = parse_macro_input!(input as ItemImpl);
    let mut errors = Vec::new();
    let mut inventory = Vec::new();

    for impl_item in &mut item.items {
        let ImplItem::Fn(method) = impl_item else {
            continue;
        };
        if !matches!(method.vis, Visibility::Public(_)) {
            continue;
        }

        let mut markers = Vec::new();
        let mut retained = Vec::new();
        for attr in method.attrs.drain(..) {
            if attr.path().is_ident("sqlite_backend") {
                markers.push(attr);
            } else {
                retained.push(attr);
            }
        }
        method.attrs = retained;

        let name = method.sig.ident.to_string();
        match markers.as_slice() {
            [] => errors.push(syn::Error::new(
                method.sig.ident.span(),
                format!("public Db method `{name}` is missing #[sqlite_backend(...)]"),
            )),
            [marker] => match parse_marker(marker) {
                Ok(reason) => inventory.push((name, reason)),
                Err(error) => errors.push(error),
            },
            _ => errors.push(syn::Error::new(
                method.sig.ident.span(),
                format!("public Db method `{name}` has duplicate #[sqlite_backend] markers"),
            )),
        }
    }

    inventory.sort_by(|a, b| a.0.cmp(&b.0));
    let names = inventory
        .iter()
        .map(|(name, _)| LitStr::new(name, proc_macro2::Span::call_site()));
    let reasons = inventory.iter().map(|(_, reason)| match reason {
        Some(reason) => quote! { Some(#reason) },
        None => quote! { None },
    });
    let errors = errors.into_iter().map(|e| e.to_compile_error());

    quote! {
        #(#errors)*
        #item
        #[doc(hidden)]
        pub const SQLITE_BACKEND_INVENTORY: &[(&str, Option<&str>)] = &[#((#names, #reasons)),*];
    }
    .into()
}

fn parse_marker(attr: &Attribute) -> syn::Result<Option<LitStr>> {
    let mut result = None;
    attr.parse_nested_meta(|meta| {
        if meta.path.is_ident("implemented") {
            if result.is_some() {
                return Err(meta.error("duplicate backend decision"));
            }
            result = Some(None);
            return Ok(());
        }
        if meta.path.is_ident("unsupported") {
            if result.is_some() {
                return Err(meta.error("duplicate backend decision"));
            }
            let value = meta.value()?.parse::<LitStr>()?;
            if value.value().trim().is_empty() {
                return Err(syn::Error::new(
                    value.span(),
                    "unsupported reason must not be empty",
                ));
            }
            result = Some(Some(value));
            return Ok(());
        }
        Err(meta.error("expected `implemented` or `unsupported = \"reason\"`"))
    })?;
    result.ok_or_else(|| {
        syn::Error::new(
            attr.span(),
            "backend decision must be `implemented` or `unsupported = \"reason\"`",
        )
    })
}

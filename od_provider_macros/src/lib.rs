//! Procedural half of `od_provider`'s inventory-backed provider macros.
//!
//! [`provider_impl`] registers a concrete implementation into the registry
//! module declared by `od_provider::define_provider!`. See the `od_provider`
//! crate docs for the full API and usage example.

use proc_macro::TokenStream;
use quote::quote;
use syn::{
    parse::{Parse, ParseStream},
    parse_macro_input, ItemImpl, LitStr, Path, Token,
};

struct ProviderArgs {
    name: LitStr,
    registry_module: Path,
}

impl Parse for ProviderArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let name: LitStr = input.parse()?;
        input.parse::<Token![for]>()?;
        let registry_module: Path = input.parse()?;

        if !input.is_empty() {
            return Err(input.error(
                "unexpected extra tokens; #[provider_impl] takes only `\"name\" for module` — \
                 the registered type is always built by deserializing the selecting config value \
                 into `Self`, so use `#[serde(skip, default = \"...\")]` on fields that can't be \
                 `Deserialize` instead of a custom constructor",
            ));
        }

        Ok(ProviderArgs {
            name,
            registry_module,
        })
    }
}

/// Registers the annotated `impl` as a named provider in the registry module
/// created by `od_provider::define_provider!`.
///
/// Takes a plain inherent `impl Type { ... }` block — no trait — and
/// synthesizes `impl <module>::Impl for Type { ... }` around it, so the call
/// site doesn't repeat the module name (already given via `for bar`) or the
/// fixed `Impl` re-export name:
///
/// ```ignore
/// od_provider::define_provider!(bar: MyTrait);
///
/// #[derive(serde::Deserialize)]
/// struct Foo {}
///
/// #[od_provider::provider_impl("foo" for bar)]
/// impl Foo {
///     // ... MyTrait's methods ...
/// }
/// ```
///
/// The provider is always built by deserializing the config value that
/// selected it directly into `Self` — the concrete type must implement
/// `serde::de::DeserializeOwned`. For fields that can't be `Deserialize`
/// (trait objects, mutexes, open handles), use ordinary serde tooling rather
/// than a custom constructor:
///
/// ```ignore
/// #[derive(serde::Deserialize)]
/// struct Foo {
///     #[serde(skip)] // uses `Default::default()`
///     cache: std::sync::Mutex<Vec<String>>,
///     #[serde(skip, default = "default_stream")]
///     stream: Box<dyn std::io::Write>,
/// }
///
/// fn default_stream() -> Box<dyn std::io::Write> {
///     Box::new(std::io::stdout())
/// }
/// ```
///
/// The generated code refers to `::od_provider::...`, so the `od_provider`
/// dependency must not be renamed in the using crate's `Cargo.toml`.
#[proc_macro_attribute]
pub fn provider_impl(args: TokenStream, item: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args as ProviderArgs);
    let item_impl = parse_macro_input!(item as ItemImpl);

    if item_impl.trait_.is_some() {
        return syn::Error::new_spanned(
            &item_impl,
            "#[provider_impl] takes a plain `impl Type { ... }` block (no trait) and \
             synthesizes `impl <module>::Impl for Type` itself — remove the trait from \
             the impl header",
        )
        .to_compile_error()
        .into();
    }

    if !item_impl.generics.params.is_empty() {
        return syn::Error::new_spanned(
            &item_impl.generics,
            "#[provider_impl] does not support generic impls; the registered type must be concrete",
        )
        .to_compile_error()
        .into();
    }

    let self_ty = &item_impl.self_ty;
    let items = &item_impl.items;
    let name = &args.name;
    let registry_module = &args.registry_module;

    let construct_fn = quote! {
        fn construct(
            value: ::od_provider::Value,
        ) -> ::od_provider::Result<Box<dyn #registry_module::Impl>> {
            let concrete: #self_ty = ::od_provider::__deserialize(value)?;
            Ok(Box::new(concrete))
        }
    };

    quote! {
        impl #registry_module::Impl for #self_ty {
            #(#items)*
        }

        const _: () = {
            #construct_fn

            ::od_provider::__inventory::submit! {
                #registry_module::Registry::new(#name, construct)
            }
        };
    }
    .into()
}

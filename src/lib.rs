//! Inventory-backed provider registries.
//!
//! [`define_provider!`] declares a small, `inventory`-backed registry module
//! for a trait; the [`provider_impl`] attribute registers a concrete
//! implementation into it, from a plain inherent `impl Type { ... }` block —
//! it synthesizes the `impl <module>::Impl for Type` header itself, so the
//! call site doesn't repeat the module name or the fixed `Impl` re-export
//! name. The module path given to both macros is the namespace shared
//! between them — it avoids deriving a registry type name from the trait
//! name, and allows nested or renamed registries.
//!
//! Every provider is constructed from a `toml::Value` — a provider is, by
//! definition, something loadable from config: the concrete type must
//! implement `Deserialize`, and it's built directly from the value that
//! selected it. Fields that can't be `Deserialize` (trait objects, mutexes,
//! open handles) use ordinary serde tooling, not a bespoke escape hatch —
//! `#[serde(skip)]` (uses `Default::default()`) or `#[serde(skip, default =
//! "...")]` for a custom default.
//!
//! The generated module also exposes a `Provider` newtype that implements
//! `Deserialize` itself, by reading a `type = "..."` tag out of the value
//! and dispatching to the matching registered implementation. It's an
//! ordinary field type — embed it anywhere in any config struct, at
//! whatever key path that struct's own `Deserialize` impl puts it.
//!
//! ```
//! use od_provider::{define_provider, provider_impl};
//! use serde::Deserialize;
//!
//! pub trait Greeter {
//!     fn greet(&self) -> String;
//! }
//!
//! define_provider!(greeters: crate::Greeter);
//!
//! #[derive(Deserialize)]
//! struct EnglishGreeter {}
//!
//! #[provider_impl("en" for greeters)]
//! impl EnglishGreeter {
//!     fn greet(&self) -> String {
//!         "hello".to_string()
//!     }
//! }
//!
//! fn main() {
//!     let provider: greeters::Provider = toml::from_str(r#"type = "en""#).unwrap();
//!     assert_eq!(provider.greet(), "hello");
//! }
//! ```
//!
//! `$trait` (for `define_provider!`) becomes the target of a `pub use`
//! inside a freshly generated module, so it needs to resolve *from there*,
//! not from the call site — a bare, unqualified trait name will not
//! resolve even if it's in scope where `define_provider!` is invoked. Use
//! `super::Trait` for a trait in the enclosing module, or (most robust) a
//! `crate::`-rooted path.
//!
//! Registration order is unspecified, and nothing here rejects duplicate
//! names; a crate that cares should check for duplicates itself, e.g. while
//! building its own lookup table at startup.
//!
//! Crates using these macros only need to depend on `od_provider` (plus
//! `serde`, for `#[derive(Deserialize)]`): the generated code reaches
//! `inventory`, `serde` and `toml` through `od_provider`'s re-exports. The
//! dependency must not be renamed in `Cargo.toml`, since `#[provider_impl]`
//! emits `::od_provider::...` paths.

// Lets `#[provider_impl]`'s generated `::od_provider::...` paths resolve even
// when the attribute is used inside this crate's own tests/doctests.
extern crate self as od_provider;

pub use od_provider_macros::provider_impl;

/// The error returned when a registered provider can't be built from the
/// config value that selected it.
pub type Error = toml::de::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[doc(hidden)]
pub use inventory as __inventory;
#[doc(hidden)]
pub use serde as __serde;
#[doc(hidden)]
pub use toml as __toml;

/// Declares an `inventory`-backed registry module named `$module` for trait
/// `$trait`.
///
/// Expands to a `pub mod $module` containing:
/// - `Impl` — a re-export of `$trait`, used as the trait-object bound for
///   registered providers. It must be an actual trait re-export (not a type
///   alias) so that downstream code can write `impl $module::Impl for Foo`.
/// - `Registry` — the `inventory`-collected type; each entry pairs a
///   provider `name` with a constructor built from a `toml::Value`. Build
///   one with [`provider_impl`] rather than by hand.
/// - `Provider` — a `Box<dyn Impl>` newtype that implements `Deserialize`:
///   it reads a `type = "..."` tag out of the value, looks up the matching
///   `Registry` entry by name, and constructs it from the same value.
///
/// Look up a provider by hand with `inventory::iter::<$module::Registry>()`,
/// or deserialize one directly via `$module::Provider`. See the
/// [crate docs](crate) for path-resolution caveats on `$trait`.
#[macro_export]
macro_rules! define_provider {
    ($module:ident : $trait:path $(,)?) => {
        pub mod $module {
            pub use $trait as Impl;

            pub struct Registry {
                pub name: &'static str,
                constructor: fn($crate::__toml::Value) -> $crate::Result<Box<dyn Impl>>,
            }

            impl Registry {
                pub const fn new(
                    name: &'static str,
                    constructor: fn($crate::__toml::Value) -> $crate::Result<Box<dyn Impl>>,
                ) -> Self {
                    Self { name, constructor }
                }

                pub fn construct(&self, value: $crate::__toml::Value) -> $crate::Result<Box<dyn Impl>> {
                    (self.constructor)(value)
                }
            }

            $crate::__inventory::collect!(Registry);

            /// A config-deserializable handle to a registered provider,
            /// selected by a `type = "..."` tag matching one of the names
            /// passed to `#[provider_impl]`. Embed this as a field anywhere
            /// in any `Deserialize` config struct.
            #[allow(dead_code)]
            pub struct Provider(pub Box<dyn Impl>);

            impl ::std::ops::Deref for Provider {
                type Target = dyn Impl;

                fn deref(&self) -> &Self::Target {
                    &*self.0
                }
            }

            impl<'de> $crate::__serde::Deserialize<'de> for Provider {
                fn deserialize<D>(deserializer: D) -> ::std::result::Result<Self, D::Error>
                where
                    D: $crate::__serde::Deserializer<'de>,
                {
                    use $crate::__serde::de::Error as _;

                    let value: $crate::__toml::Value =
                        $crate::__serde::Deserialize::deserialize(deserializer)?;
                    let tag = value
                        .get("type")
                        .and_then($crate::__toml::Value::as_str)
                        .ok_or_else(|| {
                            D::Error::custom(concat!(
                                "missing `type` field selecting a `",
                                stringify!($module),
                                "` provider"
                            ))
                        })?
                        .to_string();

                    let entry = $crate::__inventory::iter::<Registry>()
                        .find(|entry| entry.name == tag)
                        .ok_or_else(|| {
                            D::Error::custom(format!(
                                "unknown {} provider: {:?}",
                                stringify!($module),
                                tag
                            ))
                        })?;

                    entry.construct(value).map(Provider).map_err(D::Error::custom)
                }
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::provider_impl;
    use serde::Deserialize;

    // ── local trait (same module as the `define_provider!` call), default
    // constructor via `Deserialize`. Referenced as `super::Greeter`: see the
    // crate docs for why a bare `Greeter` wouldn't resolve here. ────────────

    pub trait Greeter {
        fn greet(&self) -> String;
    }

    crate::define_provider!(greeters: super::Greeter);

    #[derive(Deserialize)]
    struct EnglishGreeter {}

    #[provider_impl("en" for greeters)]
    impl EnglishGreeter {
        fn greet(&self) -> String {
            "hello".to_string()
        }
    }

    // ── a provider with its own config field, defaulted when omitted ────────

    #[derive(Deserialize)]
    struct FrenchGreeter {
        #[serde(default = "default_greeting")]
        greeting: String,
    }

    fn default_greeting() -> String {
        "bonjour".to_string()
    }

    #[provider_impl("fr" for greeters)]
    impl FrenchGreeter {
        fn greet(&self) -> String {
            self.greeting.clone()
        }
    }

    fn find_greeter(name: &str) -> Option<Box<dyn greeters::Impl>> {
        inventory::iter::<greeters::Registry>()
            .find(|entry| entry.name == name)
            .and_then(|entry| entry.construct(toml::Value::Table(Default::default())).ok())
    }

    #[test]
    fn default_constructor_resolves() {
        assert_eq!(find_greeter("en").unwrap().greet(), "hello");
    }

    #[test]
    fn field_defaults_when_omitted() {
        assert_eq!(find_greeter("fr").unwrap().greet(), "bonjour");
    }

    #[test]
    fn unknown_name_resolves_to_none() {
        assert!(find_greeter("missing").is_none());
    }

    // ── deserializing `Provider` directly from a TOML value, tag-dispatched ─

    #[test]
    fn provider_deserializes_default_constructor() {
        let provider: greeters::Provider = toml::from_str(r#"type = "en""#).unwrap();
        assert_eq!(provider.greet(), "hello");
    }

    #[test]
    fn provider_deserializes_configured_field() {
        let provider: greeters::Provider =
            toml::from_str("type = \"fr\"\ngreeting = \"salut\"").unwrap();
        assert_eq!(provider.greet(), "salut");
    }

    #[test]
    fn provider_deserialize_missing_type_errors() {
        let err = toml::from_str::<greeters::Provider>("").err().unwrap();
        assert!(err.to_string().contains("missing `type`"), "{err}");
    }

    #[test]
    fn provider_deserialize_unknown_type_errors() {
        let err = toml::from_str::<greeters::Provider>(r#"type = "missing""#)
            .err()
            .unwrap();
        assert!(err.to_string().contains("unknown"), "{err}");
    }

    #[test]
    fn provider_deserialize_bad_field_errors() {
        let err = toml::from_str::<greeters::Provider>("type = \"fr\"\ngreeting = 1")
            .err()
            .unwrap();
        assert!(err.to_string().contains("greeting"), "{err}");
    }

    // ── trait referenced via a `crate::...` path ────────────────────────────

    pub trait Speaker {
        fn speak(&self) -> &'static str;
    }

    mod nested {
        pub use super::Speaker;
    }

    crate::define_provider!(speakers: crate::tests::nested::Speaker);

    #[derive(Deserialize)]
    struct LoudSpeaker {}

    #[provider_impl("loud" for speakers)]
    impl LoudSpeaker {
        fn speak(&self) -> &'static str {
            "LOUD"
        }
    }

    #[test]
    fn crate_path_trait_resolves() {
        let found = inventory::iter::<speakers::Registry>()
            .find(|entry| entry.name == "loud")
            .and_then(|entry| entry.construct(toml::Value::Table(Default::default())).ok())
            .unwrap();
        assert_eq!(found.speak(), "LOUD");
    }

    // ── trait imported under an alias ───────────────────────────────────────

    pub use Speaker as AliasedSpeaker;

    crate::define_provider!(aliased_speakers: super::AliasedSpeaker);

    #[derive(Deserialize)]
    struct QuietSpeaker {}

    #[provider_impl("quiet" for aliased_speakers)]
    impl QuietSpeaker {
        fn speak(&self) -> &'static str {
            "quiet"
        }
    }

    #[test]
    fn aliased_trait_resolves() {
        let found = inventory::iter::<aliased_speakers::Registry>()
            .find(|entry| entry.name == "quiet")
            .and_then(|entry| entry.construct(toml::Value::Table(Default::default())).ok())
            .unwrap();
        assert_eq!(found.speak(), "quiet");
    }
}

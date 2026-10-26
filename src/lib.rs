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
//! Every provider is constructed from config — a provider is, by definition,
//! something loadable from config: the concrete type must implement
//! `Deserialize`, and it's built directly from the value that selected it.
//! Nothing here is tied to a particular format: TOML, YAML, JSON or any
//! other serde data format works, since the selecting value is buffered as a
//! format-neutral [`Value`] before being handed to the provider. Fields that can't be `Deserialize` (trait objects, mutexes,
//! open handles) use ordinary serde tooling, not a bespoke escape hatch —
//! `#[serde(skip)]` (uses `Default::default()`) or `#[serde(skip, default =
//! "...")]` for a custom default.
//!
//! The generated module also exposes a `Provider` newtype that implements
//! `Deserialize` itself, by reading a `type` tag out of the value (a map)
//! and dispatching to the matching registered implementation. The tag is
//! removed before the provider is built from the rest of the map, so
//! provider types can use `#[serde(deny_unknown_fields)]`. It's an
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
//! `serde`, for `#[derive(Deserialize)]`, and whichever format crate they
//! read config with): the generated code reaches `inventory` and `serde`
//! through `od_provider`'s re-exports. The
//! dependency must not be renamed in `Cargo.toml`, since `#[provider_impl]`
//! emits `::od_provider::...` paths.

// Lets `#[provider_impl]`'s generated `::od_provider::...` paths resolve even
// when the attribute is used inside this crate's own tests/doctests.
extern crate self as od_provider;

pub use od_provider_macros::provider_impl;

/// A format-neutral buffer of deserialized config data, holding the value
/// that selected a provider until it's handed to that provider.
pub use serde_value::Value;

/// The error returned when a registered provider can't be built from the
/// config value that selected it.
pub type Error = serde_value::DeserializerError;

pub type Result<T> = std::result::Result<T, Error>;

#[doc(hidden)]
pub use inventory as __inventory;
#[doc(hidden)]
pub use serde as __serde;

/// Builds a provider from the value that selected it, naming the offending
/// field path in the error (`serde_value`'s own errors don't carry it).
#[doc(hidden)]
pub fn __deserialize<T: serde::de::DeserializeOwned>(value: Value) -> Result<T> {
    serde_path_to_error::deserialize(value).map_err(|err| {
        let path = err.path().to_string();
        let inner = err.into_inner();
        if path == "." {
            inner
        } else {
            <Error as serde::de::Error>::custom(format!("{path}: {inner}"))
        }
    })
}

/// Declares an `inventory`-backed registry module named `$module` for trait
/// `$trait`.
///
/// Expands to a `pub mod $module` containing:
/// - `Impl` — a re-export of `$trait`, used as the trait-object bound for
///   registered providers. It must be an actual trait re-export (not a type
///   alias) so that downstream code can write `impl $module::Impl for Foo`.
/// - `Registry` — the `inventory`-collected type; each entry pairs a
///   provider `name` with a constructor built from a [`Value`]. Build
///   one with [`provider_impl`] rather than by hand.
/// - `Provider` — a `Box<dyn Impl>` newtype that implements `Deserialize`:
///   it reads a `type` tag out of the value, looks up the matching
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
                constructor: fn($crate::Value) -> $crate::Result<Box<dyn Impl>>,
            }

            impl Registry {
                pub const fn new(
                    name: &'static str,
                    constructor: fn($crate::Value) -> $crate::Result<Box<dyn Impl>>,
                ) -> Self {
                    Self { name, constructor }
                }

                pub fn construct(&self, value: $crate::Value) -> $crate::Result<Box<dyn Impl>> {
                    (self.constructor)(value)
                }
            }

            $crate::__inventory::collect!(Registry);

            /// A config-deserializable handle to a registered provider,
            /// selected by a `type` tag matching one of the names
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

                    let mut value: $crate::Value =
                        $crate::__serde::Deserialize::deserialize(deserializer)?;
                    // The tag is removed before the provider is built from
                    // the rest of the map, so it never shows up as a field.
                    let tag = match &mut value {
                        $crate::Value::Map(map) => {
                            match map.remove(&$crate::Value::String("type".to_string())) {
                                Some($crate::Value::String(tag)) => Some(tag),
                                _ => None,
                            }
                        }
                        _ => None,
                    };
                    let tag = tag
                        .ok_or_else(|| {
                            D::Error::custom(concat!(
                                "missing `type` field selecting a `",
                                stringify!($module),
                                "` provider"
                            ))
                        })?;

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
            .and_then(|entry| entry.construct(crate::Value::Map(Default::default())).ok())
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

    // ── deserializing `Provider` from a TOML value, tag-dispatched ──────────

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

    // ── other serde formats: the tag dispatch isn't tied to TOML ────────────

    #[test]
    fn provider_deserializes_from_yaml() {
        let provider: greeters::Provider =
            serde_yaml::from_str("type: fr\ngreeting: hallo").unwrap();
        assert_eq!(provider.greet(), "hallo");
    }

    #[test]
    fn provider_deserializes_from_json() {
        let provider: greeters::Provider =
            serde_json::from_str(r#"{"type": "fr", "greeting": "hola"}"#).unwrap();
        assert_eq!(provider.greet(), "hola");
    }

    #[test]
    fn provider_embeds_in_yaml_config() {
        #[derive(Deserialize)]
        struct Config {
            greeters: Vec<greeters::Provider>,
        }

        let config: Config =
            serde_yaml::from_str("greeters:\n  - type: en\n  - type: fr\n").unwrap();
        let greetings: Vec<_> = config.greeters.iter().map(|g| g.greet()).collect();
        assert_eq!(greetings, ["hello", "bonjour"]);
    }

    #[test]
    fn provider_accepts_null_and_option_fields() {
        #[derive(Deserialize)]
        struct OptionalGreeter {
            greeting: Option<String>,
        }

        #[provider_impl("opt" for greeters)]
        impl OptionalGreeter {
            fn greet(&self) -> String {
                self.greeting.clone().unwrap_or_else(|| "…".to_string())
            }
        }

        let provider: greeters::Provider =
            serde_json::from_str(r#"{"type": "opt", "greeting": null}"#).unwrap();
        assert_eq!(provider.greet(), "…");
        let provider: greeters::Provider =
            serde_yaml::from_str("type: opt\ngreeting: hi").unwrap();
        assert_eq!(provider.greet(), "hi");
    }

    #[test]
    fn provider_tag_is_not_passed_as_a_field() {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct StrictGreeter {
            greeting: String,
        }

        #[provider_impl("strict" for greeters)]
        impl StrictGreeter {
            fn greet(&self) -> String {
                self.greeting.clone()
            }
        }

        let provider: greeters::Provider =
            toml::from_str("type = \"strict\"\ngreeting = \"hey\"").unwrap();
        assert_eq!(provider.greet(), "hey");

        let err = toml::from_str::<greeters::Provider>(
            "type = \"strict\"\ngreeting = \"hey\"\nextra = 1",
        )
        .err()
        .unwrap();
        assert!(err.to_string().contains("extra"), "{err}");
    }

    #[test]
    fn provider_deserialize_non_map_errors() {
        let err = serde_json::from_str::<greeters::Provider>(r#""en""#)
            .err()
            .unwrap();
        assert!(err.to_string().contains("missing `type`"), "{err}");
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
            .and_then(|entry| entry.construct(crate::Value::Map(Default::default())).ok())
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
            .and_then(|entry| entry.construct(crate::Value::Map(Default::default())).ok())
            .unwrap();
        assert_eq!(found.speak(), "quiet");
    }
}

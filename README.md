# od_provider

Inventory-backed, config-deserializable provider registries for Rust traits.

`define_provider!` declares a registry module for a trait, and `#[provider_impl]`
registers implementations into it. The generated `Provider` type deserializes from a
TOML value by dispatching on its `type = "..."` tag.

```toml
[dependencies]
od_provider = { git = "<repo url>" }
serde = { version = "1", features = ["derive"] }
```

```rust
use od_provider::{define_provider, provider_impl};
use serde::Deserialize;

pub trait Greeter {
    fn greet(&self) -> String;
}

define_provider!(greeters: crate::Greeter);

#[derive(Deserialize)]
struct EnglishGreeter {}

#[provider_impl("en" for greeters)]
impl EnglishGreeter {
    fn greet(&self) -> String {
        "hello".to_string()
    }
}

let provider: greeters::Provider = toml::from_str(r#"type = "en""#).unwrap();
assert_eq!(provider.greet(), "hello");
```

See the crate docs (`cargo doc --open`) for path-resolution rules and caveats.

## Layout

- `src/lib.rs`: the `define_provider!` macro, re-exports, and tests
- `od_provider_macros/`: the `#[provider_impl]` attribute (a proc-macro crate)

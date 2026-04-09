[package]
name = "{{PROG_NAME}}"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib", "lib"]
name = "{{PROG_NAME}}"

[features]
no-entrypoint = []
no-idl = []
no-log-ix-name = []
cpi = ["no-entrypoint"]
default = []
idl-build = ["anchor-lang/idl-build"]

[dependencies]
anchor-lang = { version = "0.30.1", features = ["init-if-needed"] }
anchor-spl = { version = "0.30.1", features = ["token", "token_2022", "associated_token"] }
spl-token-2022 = { version = "3", features = ["no-entrypoint"] }

# Pin transitive deps to avoid edition2024 (requires Cargo 1.85+)
# Solana 1.18.26 bundled cargo is 1.75.0 which cannot parse crates using edition = "2024"
# Chain: anchor-lang-idl -> toml_edit >= 0.22.22 -> winnow >= 1.0 -> hashbrown 0.17 (edition2024)
winnow = ">=0.5, <1.0"
hashbrown = ">=0.12, <0.17"

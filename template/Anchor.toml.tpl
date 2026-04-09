[features]
resolution = true
skip-lint = false
[programs.localnet]
{{PROG_NAME}} = "{{PROG_ADDR}}"
[registry]
url = "https://api.apr.dev"
[provider]
cluster = "Localnet"
wallet = "~/.config/solana/id.json"
[workspace]
members = ["programs/deployer"]

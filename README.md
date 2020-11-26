# Bitcoin RPC Proxy

Finer-grained permission management for bitcoind.

## About

This is a proxy made specifically for `bitcoind` to allow finer-grained control of permissions. It enables you to specify several users and for each user the list of RPC calls they are allowed to make. When run against a prunded node, the proxy will perform on-demand block fetching and verification, enabling features of a non-pruned node while still using a pruned node.

### Fine-grained permission management

This is useful because `bitcoind` allows every application with password to make possibly harmful calls like stopping the daemon or spending from wallet (if enabled). If you have several applications, you can provide the less trusted ones a different password and permissions than the others using this project.

There's another interesting advantage: since this is written in Rust, it might serve as a filter for **some** malformed requests which might be exploits. But I don't recommend relying on it!

### On-demand block fetching

By connecting to your pruned Bitcoin node through Bitcoin Proxy, your node will now behave as though it is not pruned. If a user or application requires a block that is not retained by your pruned node, Bitcoin Proxy will dynamically fetch the block over the P2P network, then verify its hash against your node to ensure validity.

This means that you can run multiple services against your _pruned_ Bitcoin node — such as Lightning and BTCPay — without them fighting for control over the pruning. Both are happy because both believe they are dealing with an _unpruned_ node.

A tradeoff to the proxy is speed and bandwidth. Every time the proxy needs to fetch a block not retained by your pruned node, it must reach out over the P2P network, consuming both Internet bandwidth and time.

## Usage

For security and performance reasons this application is written in Rust. Thus, you need a recent Rust compiler to compile it.

You need to configure the proxy using config files. The application looks for files `/etc/bitcoin/rpc_proxy.toml` and `./btc_rpc_proxy.toml` and loads configuration from them, if present. **Make sure to set their permissions to `600` before you write the passwords to them!**

An example configuration file is provided in this repository, hopefuly it's understandable. After configuring, you only need to run the compiled binary (e.g. using `cargo run --release`)

A man page is also generated during build and `--help` option is provided.

## Limitations

* It uses `serde_json`, which allocates during deserialization (`Value`). Expect a bit lower performance than without proxy.
* Logging can't be configured yet.
* No support for changing UID.
* No support for Unix sockets.
* Redirect instead of blocking might be a useful feaure, which is now lacking.

License
-------

MITNFA

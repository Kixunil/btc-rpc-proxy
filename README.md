Bitcoin RPC proxy
=================

Finer-grained permission management for bitcoind.

About
-----

This is a proxy made specifically for `bitcoind` to allow finer-grained control of permissions. It enables you to specify several users and for each user the list of RPC calls he's allowed to make.

This is useful because `bitcoind` allows every application with password to make possibly harmful calls like stopping the daemon or spending from wallet (if enabled). If you have several applications, you can provide the less trusted ones a different password and permissions than the others using this project.

There's another interesting advantage: since this is written in Rust, it might serve as a filter for **some** malformed requests which might be exploits. But I don't recommend relying on it!

Usage
-----

For security and performance reasons this application is written in Rust. Thus, you need a recent Rust compiler to compile it.

You need to configure the proxy using config files. The application looks for files `/etc/bitcoin/rpc_proxy.toml` and `./btc_rpc_proxy.toml` and loads configuration from them, if present. **Make sure to set their permissions to `600` before you write the passwords to them!**

An example configuration file is provided in this repository, hopefuly it's understandable. After configuring, you only need to run the compiled binary (e.g. using `cargo run --release`)

A man page is also generated during build and `--help` option is provided.

Limitations
-----------

**BIG FAT WARNING: this project is very young and hasn't been reviewed yet! Use at your own risk! The author reserves the right to publicly ridicule anyone stupid enough to blindly run this!**

Aside the project being young, there are some other issues:

* It uses `serde_json`, which allocates during deserialization (`Value`). Expect a bit lower performance than without proxy.
* Logging can't be configured yet.
* No support for changing UID.
* No support for Unix sockets.
* Redirect instead of blocking might be a useful feaure, which is now lacking.
* The quality of the code shouldn't be too bad, but I wouldn't call it "high".

License
-------

MITNFA

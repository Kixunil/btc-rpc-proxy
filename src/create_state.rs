use std::sync::Arc;
use std::time::Duration;

use anyhow::Error;
use btc_rpc_proxy::{AuthSource, Peers, RpcClient, State, TorState};
use slog::Drain;
use tokio::sync::RwLock;

#[allow(dead_code)]
#[allow(unused_mut)]
#[allow(unused_variables)]
mod config {
    include!(concat!(env!("OUT_DIR"), "/configure_me_config.rs"));
}
use self::config::{Config, ResultExt};

pub fn create_state() -> Result<State, Error> {
    use slog::Level;

    let (config, _) =
        Config::including_optional_config_files(std::iter::empty::<&str>()).unwrap_or_exit();

    let log_level = match config.verbose {
        0 => Level::Critical,
        1 => Level::Error,
        2 => Level::Warning,
        3 => Level::Info,
        4 => Level::Debug,
        _ => Level::Trace,
    };

    let decorator = slog_term::TermDecorator::new()
        .stderr()
        .build();
    let drain = slog_term::FullFormat::new(decorator).build().fuse();
    let drain = slog_async::Async::new(drain).build().filter_level(log_level).fuse();
    let logger = slog::Logger::root(drain, slog::o!());

    let auth = AuthSource::from_config(
        config.bitcoind_user,
        config.bitcoind_password,
        config.cookie_file,
    )?;
    let bitcoin_uri = format!(
        "http://{}:{}/",
        config.bitcoind_address, config.bitcoind_port
    )
    .parse()
    .map_err(|error: http::uri::InvalidUri| {
        let new_error = anyhow::anyhow!("failed to parse bitcoin URI: {}", error);
        slog::error!(logger, "failed to parse bitcoin URI"; "error" => #error);
        new_error
    })?;
    let rpc_client = RpcClient::new(auth, bitcoin_uri, &logger);

    let tor_only = config.tor_only;
    let tor = config.tor_proxy.map(|proxy| TorState {
        proxy,
        only: tor_only,
    });

    Ok(State {
        bind: (config.bind_address, config.bind_port).into(),
        rpc_client,
        tor,
        users: btc_rpc_proxy::users::input::map_default(config.user, config.default_fetch_blocks),
        logger,
        peer_timeout: Duration::from_secs(config.peer_timeout),
        peers: RwLock::new(Arc::new(Peers::new())),
        max_peer_age: Duration::from_secs(config.max_peer_age),
        max_peer_concurrency: config.max_peer_concurrency,
    })
}

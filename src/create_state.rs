use std::time::Duration;

use anyhow::Error;
use btc_rpc_proxy::{AuthSource, Peers, RpcClient, State, TorState, Users};
use slog::Drain;
use tokio::sync::Mutex;

use config::ResultExt;

include_config!();

pub fn create_state() -> Result<State, Error> {
    let (config, _) = config::Config::including_optional_config_files(std::iter::empty::<&str>())
        .unwrap_or_exit();

    let auth = AuthSource::from_config(
        config.bitcoind_user,
        config.bitcoind_password,
        config.cookie_file,
    )?;
    let bitcoin_uri = format!(
        "http://{}:{}/",
        config.bitcoind_address, config.bitcoind_port
    )
    .parse()?;
    let rpc_client = RpcClient::new(auth, bitcoin_uri)?;

    let tor_only = config.tor_only;
    let tor = config.tor_proxy.map(|proxy| TorState {
        proxy,
        only: tor_only,
    });

    let decorator = slog_term::TermDecorator::new().build();
    let drain = slog_term::FullFormat::new(decorator).build().fuse();
    let drain = slog_async::Async::new(drain).build().fuse();
    let logger = slog::Logger::root(drain, slog::o!());

    Ok(State {
        bind: (config.bind_address, config.bind_port).into(),
        rpc_client,
        tor,
        users: Users(config.user),
        logger,
        peer_timeout: Duration::from_secs(config.peer_timeout),
        peers: Mutex::new(Peers::new()),
        max_peer_age: Duration::from_secs(config.max_peer_age),
    })
}

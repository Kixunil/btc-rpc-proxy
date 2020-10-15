use std::net::IpAddr;
use std::time::Duration;

use anyhow::Error;
use btc_rpc_proxy::{util::deserialize_parse, AuthSource, Env, PeerList, RpcClient, Users};
use http::uri;
use hyper::Uri;
use slog::Drain;
use tokio::sync::Mutex;

#[derive(serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
struct Config {
    pub bitcoind: BitcoinCoreConfig,
    pub users: Users,
    pub advanced: AdvancedConfig,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
struct AdvancedConfig {
    pub tor_only: bool,
    pub peer_timeout: u64,
    pub max_peer_age: u64,
}

#[derive(serde::Deserialize)]
#[serde(tag = "type")]
#[serde(rename_all = "kebab-case")]
enum BitcoinCoreConfig {
    Internal {
        address: IpAddr,
        user: String,
        password: String,
    },
    External {
        #[serde(deserialize_with = "deserialize_parse")]
        address: Uri,
        user: String,
        password: String,
    },
    QuickConnect {
        #[serde(deserialize_with = "deserialize_parse")]
        quick_connect_url: Uri,
    },
}

#[derive(serde::Serialize)]
pub struct Properties {
    version: u8,
    data: (Property<Vec<Property<String>>>,),
}

#[derive(serde::Serialize)]
pub struct Property<T> {
    name: String,
    value: T,
    description: Option<String>,
    copyable: bool,
    qr: bool,
}

pub async fn create_env() -> Result<Env, Error> {
    let cfg: Config = tokio::task::spawn_blocking(move || -> Result<_, Error> {
        let cfg: Config =
            serde_yaml::from_reader(std::fs::File::open("/root/start9/config.yaml")?)?;
        let tor_addr = std::env::var("TOR_ADDRESS")?;
        serde_yaml::to_writer(
            std::fs::File::create("/root/start9/stats.yaml")?,
            &Properties {
                version: 1,
                data: (Property {
                    name: "Quick Connect URLs".to_owned(),
                    value: cfg
                        .users
                        .0
                        .iter()
                        .map(|(name, info)| Property {
                            name: name.clone(),
                            value: format!(
                                "btcstandup://{}:{}@{}:8332/",
                                name, info.password, tor_addr
                            ),
                            description: Some(format!("Quick Connect URL for {}", name)),
                            copyable: true,
                            qr: true,
                        })
                        .collect(),
                    description: Some("Quick Connect URLs for each user".to_owned()),
                    copyable: false,
                    qr: false,
                },),
            },
        )?;
        Ok(cfg)
    })
    .await??;
    let decorator = slog_term::TermDecorator::new().build();
    let drain = slog_term::FullFormat::new(decorator).build().fuse();
    let drain = slog_async::Async::new(drain).build().fuse();
    let logger = slog::Logger::root(drain, slog::o!());
    Ok(Env {
        bind: ([0, 0, 0, 0], 8332).into(),
        rpc_client: match cfg.bitcoind {
            BitcoinCoreConfig::Internal {
                address,
                user,
                password,
            } => RpcClient::new(
                AuthSource::Const {
                    username: user,
                    password,
                },
                format!("http://{}:8332/", address).parse()?,
            )?,
            BitcoinCoreConfig::External {
                address,
                user,
                password,
            } => RpcClient::new(
                AuthSource::Const {
                    username: user,
                    password,
                },
                Uri::from_parts({
                    let mut addr = address.into_parts();
                    addr.scheme = Some(uri::Scheme::HTTP);
                    addr.path_and_query = None;
                    if let Some(ref auth) = addr.authority {
                        if auth.port().is_none() {
                            addr.authority = Some(format!("{}:8332", auth).parse()?);
                        }
                    }
                    addr
                })?,
            )?,
            BitcoinCoreConfig::QuickConnect { quick_connect_url } => {
                let auth = quick_connect_url
                    .authority()
                    .ok_or_else(|| anyhow::anyhow!("invalid Quick Connect URL"))?;
                let mut auth_split = auth.as_str().split(|c| c == ':' || c == '@');
                let user = auth_split.next().map(|s| s.to_owned());
                let password = auth_split.next().map(|s| s.to_owned());
                RpcClient::new(
                    AuthSource::from_config(user, password, None)?,
                    format!(
                        "http://{}:{}/",
                        auth.host(),
                        auth.port_u16().unwrap_or(8332)
                    )
                    .parse()?,
                )?
            }
        },
        tor_proxy: format!("{}:9050", std::env::var("HOST_IP")?).parse()?,
        tor_only: cfg.advanced.tor_only,
        users: cfg.users,
        logger,
        peer_timeout: Duration::from_secs(cfg.advanced.peer_timeout),
        peers: Mutex::new(PeerList::new()),
        max_peer_age: Duration::from_secs(cfg.advanced.max_peer_age),
    })
}

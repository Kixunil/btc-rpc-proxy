use std::net::SocketAddr;
use std::time::Duration;

use anyhow::Error;
use slog::Logger;
use tokio::sync::Mutex;

use crate::client::RpcClient;
use crate::fetch_blocks::{PeerHandle, PeerList};
use crate::users::Users;

#[derive(Debug)]
pub struct TorEnv {
    pub proxy: SocketAddr,
    pub only: bool,
}

#[derive(Debug)]
pub struct Env {
    pub bind: SocketAddr,
    pub rpc_client: RpcClient,
    pub tor: Option<TorEnv>,
    pub users: Users,
    pub logger: Logger,
    pub peer_timeout: Duration,
    pub peers: Mutex<PeerList>,
    pub max_peer_age: Duration,
}
impl Env {
    pub fn leak(self) -> &'static Self {
        Box::leak(Box::new(self))
    }
    pub async fn get_peers(&self) -> Result<Vec<PeerHandle>, Error> {
        self.peers.lock().await.get_peers(self).await
    }
}

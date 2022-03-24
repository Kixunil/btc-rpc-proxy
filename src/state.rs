use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Error;
use slog::Logger;
use tokio::sync::RwLock;

use crate::client::RpcClient;
use crate::fetch_blocks::{PeerHandle, Peers};
use crate::users::Users;

#[derive(Debug)]
pub struct TorState {
    pub proxy: SocketAddr,
    pub only: bool,
}

#[derive(Debug)]
pub struct State {
    pub rpc_client: RpcClient,
    pub tor: Option<TorState>,
    pub users: Users,
    pub logger: Logger,
    pub peer_timeout: Duration,
    pub peers: RwLock<Arc<Peers>>,
    pub max_peer_age: Duration,
    pub max_peer_concurrency: Option<usize>,
}
impl State {
    pub fn leak(self) -> &'static Self {
        Box::leak(Box::new(self))
    }
    pub fn arc(self) -> Arc<Self> {
        Arc::new(self)
    }
    pub async fn get_peers(self: Arc<Self>) -> Result<Vec<PeerHandle>, Error> {
        let mut peers = self.peers.read().await.clone();
        if peers.stale(self.max_peer_age) {
            let handle = tokio::task::spawn(async move {
                match Peers::updated(&self.rpc_client).await {
                    Ok(peers) => {
                        let res = Arc::new(peers);
                        *self.peers.write().await = res.clone();
                        Ok(res)
                    }
                    Err(error) => {
                        error!(self.logger, "failed to update peers"; "error" => #%error);
                        Err(error)
                    }
                }
            });
            if peers.is_empty() {
                peers = handle.await??;
            }
        }
        Ok(peers.handles())
    }
}

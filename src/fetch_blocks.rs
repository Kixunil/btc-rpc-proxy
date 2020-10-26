use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Error;
use bitcoin::{
    consensus::{Decodable, Encodable},
    hash_types::BlockHash,
    network::{
        address::Address,
        constants::{Network::Bitcoin, ServiceFlags},
        message::{NetworkMessage, RawNetworkMessage},
        message_blockdata::Inventory,
        message_network::VersionMessage,
    },
    Block,
};
use futures::channel::mpsc;
use futures::FutureExt;
use socks::Socks5Stream;

use crate::client::{
    GetBlock, GetBlockParams, GetPeerInfo, RpcError, RpcRequest, MISC_ERROR_CODE,
    PRUNE_ERROR_MESSAGE,
};
use crate::env::{Env, TorEnv};

type VersionMessageProducer = Box<dyn Fn(Address) -> RawNetworkMessage + Send + Sync>;

lazy_static::lazy_static! {
    static ref VER_ACK: RawNetworkMessage = RawNetworkMessage {
        magic: Bitcoin.magic(),
        payload: NetworkMessage::Verack,
    };
    static ref VERSION_MESSAGE: VersionMessageProducer = Box::new(|addr| {
        use std::time::SystemTime;
        RawNetworkMessage {
            magic: Bitcoin.magic(),
            payload: NetworkMessage::Version(VersionMessage::new(
                ServiceFlags::NONE,
                SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_secs() as i64,
                addr,
                Address::new(&([127, 0, 0, 1], 8332).into(), ServiceFlags::NONE),
                rand::random(),
                format!("BTC RPC Proxy v{}", env!("CARGO_PKG_VERSION")),
                0,
            )),
        }
    });
}

#[derive(Debug)]
pub struct PeerList {
    fetched: Option<Instant>,
    peers: Vec<Peer>,
}
impl PeerList {
    pub fn new() -> Self {
        PeerList {
            fetched: None,
            peers: Vec::new(),
        }
    }
    pub async fn get_peers(&mut self, env: &Env) -> Result<Vec<PeerHandle>, Error> {
        if self
            .fetched
            .map(|f| f.elapsed() > env.max_peer_age)
            .unwrap_or(true)
        {
            self.peers = env
                .rpc_client
                .call(&RpcRequest {
                    id: None,
                    method: GetPeerInfo,
                    params: [],
                })
                .await?
                .into_result()?
                .into_iter()
                .filter(|p| !p.inbound)
                .filter(|p| p.servicesnames.contains("NETWORK"))
                .map(|p| p.into_address().map(Peer::new))
                .collect::<Result<_, _>>()?;
            self.fetched = Some(Instant::now());
        }
        Ok(self.peers.iter_mut().map(|p| p.peer_handle()).collect())
    }
}

pub enum BitcoinPeerConnection {
    ClearNet(TcpStream),
    Tor(Socks5Stream),
}
impl Read for BitcoinPeerConnection {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            BitcoinPeerConnection::ClearNet(a) => a.read(buf),
            BitcoinPeerConnection::Tor(a) => a.read(buf),
        }
    }
}
impl Write for BitcoinPeerConnection {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            BitcoinPeerConnection::ClearNet(a) => a.write(buf),
            BitcoinPeerConnection::Tor(a) => a.write(buf),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            BitcoinPeerConnection::ClearNet(a) => a.flush(),
            BitcoinPeerConnection::Tor(a) => a.flush(),
        }
    }
}
impl BitcoinPeerConnection {
    pub async fn connect(env: Arc<Env>, addr: Address) -> Result<Self, Error> {
        tokio::time::timeout(
            env.peer_timeout,
            tokio::task::spawn_blocking(move || {
                let mut stream = match (addr.socket_addr(), &env.tor) {
                    (Ok(addr), Some(TorEnv { only: false, .. })) | (Ok(addr), None) => {
                        BitcoinPeerConnection::ClearNet(TcpStream::connect(addr)?)
                    }
                    (Ok(addr), Some(tor)) => {
                        BitcoinPeerConnection::Tor(Socks5Stream::connect(tor.proxy, addr)?)
                    }
                    (Err(_), Some(tor)) => BitcoinPeerConnection::Tor(Socks5Stream::connect(
                        tor.proxy,
                        (
                            format!(
                                "{}.onion",
                                base32::encode(
                                    base32::Alphabet::RFC4648 { padding: false },
                                    &addr
                                        .address
                                        .iter()
                                        .map(|n| *n)
                                        .flat_map(|n| u16::to_be_bytes(n).to_vec())
                                        .collect::<Vec<_>>()
                                )
                                .to_lowercase()
                            )
                            .as_str(),
                            addr.port,
                        ),
                    )?),
                    (Err(e), None) => return Err(e.into()),
                };
                VERSION_MESSAGE(addr).consensus_encode(&mut stream)?;
                stream.flush()?;
                let _ =
                    bitcoin::network::message::RawNetworkMessage::consensus_decode(&mut stream)?; // version
                let _ =
                    bitcoin::network::message::RawNetworkMessage::consensus_decode(&mut stream)?; // verack
                VER_ACK.consensus_encode(&mut stream)?;
                stream.flush()?;

                Ok(stream)
            }),
        )
        .await??
    }
}

pub struct Peer {
    addr: Arc<Address>,
    send: mpsc::Sender<BitcoinPeerConnection>,
    recv: mpsc::Receiver<BitcoinPeerConnection>,
}
impl Peer {
    pub fn new(addr: Address) -> Self {
        let (send, recv) = mpsc::channel(1);
        Peer {
            addr: Arc::new(addr),
            send,
            recv,
        }
    }
    pub fn peer_handle<'a>(&'a mut self) -> PeerHandle {
        PeerHandle {
            addr: self.addr.clone(),
            send: self.send.clone(),
            connection: self.recv.try_next().ok().and_then(|a| a),
            dirty: false,
        }
    }
}
impl std::fmt::Debug for Peer {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.debug_struct("Peer").field("addr", &self.addr).finish()
    }
}

pub struct PeerHandle {
    addr: Arc<Address>,
    send: mpsc::Sender<BitcoinPeerConnection>,
    connection: Option<BitcoinPeerConnection>,
    dirty: bool,
}
impl PeerHandle {
    pub async fn with_connection<
        T,
        F: FnOnce(&mut BitcoinPeerConnection) -> Fut,
        Fut: std::future::Future<Output = Result<T, Error>>,
    >(
        &mut self,
        env: Arc<Env>,
        f: F,
    ) -> Result<T, Error> {
        let conn = if let Some(ref mut connection) = self.connection {
            connection
        } else {
            self.connection =
                Some(BitcoinPeerConnection::connect(env, (&*self.addr).clone()).await?);
            self.connection.as_mut().unwrap()
        };
        self.dirty = true;
        f(conn).await
    }

    pub async fn with_connection_blocking<
        T: Send + 'static,
        F: FnOnce(&mut BitcoinPeerConnection) -> Result<T, Error> + Send + 'static,
    >(
        &mut self,
        env: Arc<Env>,
        f: F,
    ) -> Result<T, Error> {
        let mut conn = if let Some(connection) = self.connection.take() {
            connection
        } else {
            BitcoinPeerConnection::connect(env, (&*self.addr).clone()).await?
        };
        self.dirty = true;
        let (conn, res) = tokio::task::spawn_blocking(move || {
            let res = f(&mut conn);
            (conn, res)
        })
        .await?;
        self.connection = Some(conn);
        Ok(res?)
    }

    pub fn mark_clean(&mut self) {
        self.dirty = false;
    }
}
impl<'a> Drop for PeerHandle {
    fn drop(&mut self) {
        if !self.dirty {
            if let Some(connection) = self.connection.take() {
                self.send.try_send(connection).unwrap_or_default()
            }
        }
    }
}

async fn fetch_block_from_self(env: &Env, hash: BlockHash) -> Result<Option<Block>, RpcError> {
    match env
        .rpc_client
        .call(&RpcRequest {
            id: None,
            method: GetBlock,
            params: GetBlockParams(hash, Some(0)),
        })
        .await?
        .into_result()
    {
        Ok(b) => Ok(Some(
            Block::consensus_decode(&mut std::io::Cursor::new(
                b.as_left()
                    .ok_or_else(|| anyhow::anyhow!("unexpected response for getblock"))?
                    .as_ref(),
            ))
            .map_err(Error::from)?,
        )),
        Err(e) if e.code == MISC_ERROR_CODE && e.message == PRUNE_ERROR_MESSAGE => Ok(None),
        Err(e) => Err(e),
    }
}

async fn fetch_block_from_peer<'a>(
    env: Arc<Env>,
    hash: BlockHash,
    mut peer: PeerHandle,
) -> Result<Block, Error> {
    tokio::time::timeout(env.peer_timeout, async move {
        peer.with_connection_blocking(env.clone(), move |conn| {
            RawNetworkMessage {
                magic: Bitcoin.magic(),
                payload: NetworkMessage::GetData(vec![Inventory::Block(hash)]),
            }
            .consensus_encode(conn)
            .map_err(From::from)
        })
        .await?;

        loop {
            let msg = peer
                .with_connection_blocking(env.clone(), |conn| {
                    RawNetworkMessage::consensus_decode(conn).map_err(From::from)
                })
                .await?;
            match msg.payload {
                NetworkMessage::Block(b) => {
                    let returned_hash = b.block_hash();
                    let merkle_check = b.check_merkle_root();
                    let witness_check = b.check_witness_commitment();
                    return match (returned_hash == hash, merkle_check, witness_check) {
                        (true, true, true) => {
                            peer.mark_clean();
                            Ok(b)
                        }
                        (true, true, false) => {
                            Err(anyhow::anyhow!("Witness check failed for {:?}", hash))
                        }
                        (true, false, _) => {
                            Err(anyhow::anyhow!("Merkle check failed for {:?}", hash))
                        }
                        (false, _, _) => Err(anyhow::anyhow!(
                            "Expected block hash {:?}, got {:?}",
                            hash,
                            returned_hash
                        )),
                    };
                }
                NetworkMessage::Ping(p) => {
                    peer.with_connection_blocking(env.clone(), move |conn| {
                        RawNetworkMessage {
                            magic: Bitcoin.magic(),
                            payload: NetworkMessage::Pong(p),
                        }
                        .consensus_encode(conn)
                        .map_err(From::from)
                    })
                    .await?;
                }
                m => warn!(env.logger, "Invalid Message Received: {:?}", m),
            }
        }
    })
    .await?
}

async fn fetch_block_from_peer_set(
    env: Arc<Env>,
    peers: Vec<PeerHandle>,
    hash: BlockHash,
) -> Option<Block> {
    use futures::stream::StreamExt;

    let (send, mut recv) = futures::channel::mpsc::channel(1);
    let fut_unordered: futures::stream::FuturesUnordered<_> =
        peers.into_iter().map(futures::future::ready).collect();
    let env_local = env.clone();
    let runner = fut_unordered
        .then(move |p| fetch_block_from_peer(env_local.clone(), hash.clone(), p))
        .for_each_concurrent(None, |block_res| {
            match block_res {
                Ok(block) => send.clone().try_send(block).unwrap_or_default(),
                Err(e) => warn!(env.logger, "Error fetching block from peer: {}", e),
            }
            futures::future::ready(())
        });
    let b = futures::select! {
        b = recv.next().fuse() => b,
        _ = runner.boxed().fuse() => None,
    };
    b
}

pub async fn fetch_block(
    env: Arc<Env>,
    peers: Vec<PeerHandle>,
    hash: BlockHash,
) -> Result<Option<Block>, RpcError> {
    Ok(match fetch_block_from_self(&*env, hash).await? {
        Some(block) => Some(block),
        None => {
            info!(
                env.logger,
                "Block is pruned from Core, attempting fetch from peers."
            );
            if let Some(block) = fetch_block_from_peer_set(env.clone(), peers, hash).await {
                Some(block)
            } else {
                warn!(env.logger, "Could not fetch block from peers.");
                None
            }
        }
    })
}

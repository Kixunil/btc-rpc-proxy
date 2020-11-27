use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;
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

use crate::client::{RpcClient, RpcError, RpcRequest, MISC_ERROR_CODE, PRUNE_ERROR_MESSAGE};
use crate::env::{Env, TorEnv};
use crate::rpc_methods::{GetBlock, GetBlockParams, GetPeerInfo};

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
pub struct Peers {
    fetched: Option<Instant>,
    peers: Vec<Peer>,
}
impl Peers {
    pub fn new() -> Self {
        Peers {
            fetched: None,
            peers: Vec::new(),
        }
    }
    pub async fn get_peers(
        &mut self,
        client: &RpcClient,
        max_peer_age: Duration,
    ) -> Result<Vec<PeerHandle>, Error> {
        if self
            .fetched
            .map(|f| f.elapsed() > max_peer_age)
            .unwrap_or(true)
        {
            self.peers = client
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
        Ok(self.peers.iter_mut().map(|p| p.handle()).collect())
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
    addr: Address,
    send: mpsc::Sender<BitcoinPeerConnection>,
    recv: mpsc::Receiver<BitcoinPeerConnection>,
}
impl Peer {
    pub fn new(addr: Address) -> Self {
        let (send, recv) = mpsc::channel(1);
        Peer { addr, send, recv }
    }
    pub fn handle(&mut self) -> PeerHandle {
        PeerHandle {
            addr: self.addr.clone(),
            conn: self.recv.try_next().ok().and_then(|a| a),
            send: self.send.clone(),
        }
    }
}
impl std::fmt::Debug for Peer {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.debug_struct("Peer").field("addr", &self.addr).finish()
    }
}

pub struct PeerHandle {
    addr: Address,
    conn: Option<BitcoinPeerConnection>,
    send: mpsc::Sender<BitcoinPeerConnection>,
}
impl PeerHandle {
    pub async fn connect(&mut self, env: Arc<Env>) -> Result<RecyclableConnection, Error> {
        if let Some(conn) = self.conn.take() {
            Ok(RecyclableConnection {
                conn,
                send: self.send.clone(),
            })
        } else {
            Ok(RecyclableConnection {
                conn: BitcoinPeerConnection::connect(env, (&self.addr).clone()).await?,
                send: self.send.clone(),
            })
        }
    }
}

pub struct RecyclableConnection {
    conn: BitcoinPeerConnection,
    send: mpsc::Sender<BitcoinPeerConnection>,
}
impl RecyclableConnection {
    fn recycle(mut self) {
        self.send.try_send(self.conn).unwrap_or_default()
    }
}
impl std::ops::Deref for RecyclableConnection {
    type Target = BitcoinPeerConnection;
    fn deref(&self) -> &Self::Target {
        &self.conn
    }
}
impl std::ops::DerefMut for RecyclableConnection {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.conn
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
    mut conn: RecyclableConnection,
) -> Result<(Block, RecyclableConnection), Error> {
    tokio::time::timeout(env.peer_timeout, async move {
        conn = tokio::task::spawn_blocking(move || {
            RawNetworkMessage {
                magic: Bitcoin.magic(),
                payload: NetworkMessage::GetData(vec![Inventory::Block(hash)]),
            }
            .consensus_encode(&mut *conn)
            .map_err(Error::from)
            .map(|_| conn)
        })
        .await??;

        loop {
            let (msg, conn_) = tokio::task::spawn_blocking(move || {
                RawNetworkMessage::consensus_decode(&mut *conn)
                    .map_err(Error::from)
                    .map(|msg| (msg, conn))
            })
            .await??;
            conn = conn_;
            match msg.payload {
                NetworkMessage::Block(b) => {
                    let returned_hash = b.block_hash();
                    let merkle_check = b.check_merkle_root();
                    let witness_check = b.check_witness_commitment();
                    return match (returned_hash == hash, merkle_check, witness_check) {
                        (true, true, true) => Ok((b, conn)),
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
                    conn = tokio::task::spawn_blocking(move || {
                        RawNetworkMessage {
                            magic: Bitcoin.magic(),
                            payload: NetworkMessage::Pong(p),
                        }
                        .consensus_encode(&mut *conn)
                        .map_err(Error::from)
                        .map(|_| conn)
                    })
                    .await??;
                }
                m => warn!(env.logger, "Invalid Message Received: {:?}", m),
            }
        }
    })
    .await?
}

async fn fetch_block_from_peers(
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
        .then(move |mut peer| {
            let env_local = env_local.clone();
            async move {
                fetch_block_from_peer(
                    env_local.clone(),
                    hash.clone(),
                    peer.connect(env_local).await?,
                )
                .await
            }
        })
        .for_each_concurrent(None, |block_res| {
            match block_res {
                Ok((block, conn)) => {
                    conn.recycle();
                    send.clone().try_send(block).unwrap_or_default();
                }
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
            debug!(
                env.logger,
                "Block is pruned from Core, attempting fetch from peers."
            );
            if let Some(block) = fetch_block_from_peers(env.clone(), peers, hash).await {
                Some(block)
            } else {
                warn!(env.logger, "Could not fetch block from peers.");
                None
            }
        }
    })
}

use bitcoin::hash_types::BlockHash;
use linear_map::{set::LinearSet, LinearMap};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::client::RpcMethod;
use crate::util::{Either, HexBytes};

#[cfg(feature = "old_rust")]
use crate::util::old_rust::StrCompat;

#[derive(Debug)]
pub struct GetBlock;
#[derive(Debug, Deserialize, Serialize)]
pub struct GetBlockParams(
    pub BlockHash,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub Option<u8>,
);
impl RpcMethod for GetBlock {
    type Params = GetBlockParams;
    type Response = Either<HexBytes, GetBlockResult>;
    fn as_str(&self) -> &'static str {
        "getblock"
    }
}
impl Serialize for GetBlock {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.as_str().serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for GetBlock {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s: &'de str = Deserialize::deserialize(deserializer)?;
        if s == Self.as_str() {
            Ok(Self)
        } else {
            Err(serde::de::Error::invalid_value(
                serde::de::Unexpected::Str(s),
                &Self.as_str(),
            ))
        }
    }
}

#[derive(Debug)]
pub struct GetBlockHeader;
#[derive(Debug, Deserialize, Serialize)]
pub struct GetBlockHeaderParams(
    pub BlockHash,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub Option<bool>,
);
impl RpcMethod for GetBlockHeader {
    type Params = GetBlockHeaderParams;
    type Response = Either<HexBytes, GetBlockHeaderResult>;
    fn as_str(&self) -> &'static str {
        "getblockheader"
    }
}
impl Serialize for GetBlockHeader {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.as_str().serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for GetBlockHeader {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s: &'de str = Deserialize::deserialize(deserializer)?;
        if s == Self.as_str() {
            Ok(Self)
        } else {
            Err(serde::de::Error::invalid_value(
                serde::de::Unexpected::Str(s),
                &Self.as_str(),
            ))
        }
    }
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GetBlockHeaderResult {
    pub hash: bitcoin::BlockHash,
    pub confirmations: u32,
    pub height: usize,
    pub version: i32,
    pub version_hex: Option<HexBytes>,
    pub merkleroot: bitcoin::TxMerkleNode,
    pub time: usize,
    pub mediantime: Option<usize>,
    pub nonce: u32,
    pub bits: String,
    pub difficulty: f64,
    pub chainwork: HexBytes,
    pub n_tx: usize,
    pub previousblockhash: Option<bitcoin::BlockHash>,
    pub nextblockhash: Option<bitcoin::BlockHash>,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GetBlockResult {
    #[serde(flatten)]
    pub header: GetBlockHeaderResult,
    pub size: usize,
    pub strippedsize: Option<usize>,
    pub weight: usize,
    pub tx: Vec<bitcoin::Txid>,
}

#[derive(Debug)]
pub struct GetPeerInfo;

// https://docs.rs/bitcoincore-rpc-json/0.12.0/bitcoincore_rpc_json/struct.GetPeerInfoResult.html
#[derive(Debug, Deserialize, Serialize)]
pub struct PeerInfo {
    /// Peer index
    pub id: u64,
    /// The IP address and port of the peer
    pub addr: String,
    /// Bind address of the connection to the peer
    pub addrbind: String,
    /// Local address as reported by the peer
    pub addrlocal: Option<String>,
    /// The services offered
    // TODO: use a type for services
    pub services: String,
    /// The services offered
    pub servicesnames: LinearSet<String>,
    /// The peer version, such as 70001
    pub version: u64,
    /// The string version
    pub subver: String,
    /// Inbound (true) or Outbound (false)
    pub inbound: bool,
    /// The starting height (block) of the peer
    pub startingheight: i64,
    /// The last header we have in common with this peer
    pub synced_headers: i64,
    /// The last block we have in common with this peer
    pub synced_blocks: i64,
    /// The heights of blocks we're currently asking from this peer
    pub inflight: Vec<u64>,
}

#[derive(Debug, thiserror::Error)]
pub enum PeerAddressError {
    #[error("invalid hexadecimal encoding of {string}")]
    InvalidHex {
        string: String,
        #[source]
        error: hex::FromHexError,
    },
    #[error("can't consensus-decode {string}")]
    ConsensusDecode {
        string: String,
        #[source]
        error: bitcoin::consensus::encode::Error,
    },
    #[error("missing port in peer address {0}")]
    MissingPort(String),
    #[error("invalid port in address {address}")]
    InvalidPort {
        address: String,
        #[source]
        error: std::num::ParseIntError,
    },
    #[error("the peer address {0} is neither clearnet address nor onion address")]
    Unknown(String),
    #[error("base32 encoding of onion address {0} is invalid")]
    InvalidOnionEncoding(String),
    #[error("invalid length of onion address {0}")]
    InvalidOnionLength(String),
}

impl RpcMethod for GetPeerInfo {
    type Params = [(); 0];
    type Response = Vec<PeerInfo>;
    fn as_str(&self) -> &'static str {
        "getpeerinfo"
    }
}
impl Serialize for GetPeerInfo {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.as_str().serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for GetPeerInfo {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s: &'de str = Deserialize::deserialize(deserializer)?;
        if s == Self.as_str() {
            Ok(Self)
        } else {
            Err(serde::de::Error::invalid_value(
                serde::de::Unexpected::Str(s),
                &Self.as_str(),
            ))
        }
    }
}

#[derive(Debug)]
pub struct GetBlockchainInfo;

// https://docs.rs/bitcoincore-rpc-json/0.12.0/src/bitcoincore_rpc_json/lib.rs.html#551-557
#[derive(Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Bip9SoftforkStatus {
    Defined,
    Started,
    LockedIn,
    Active,
    Failed,
}

// https://docs.rs/bitcoincore-rpc-json/0.12.0/src/bitcoincore_rpc_json/lib.rs.html#560-566
#[derive(Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Bip9SoftforkStatistics {
    pub period: u32,
    pub threshold: u32,
    pub elapsed: u32,
    pub count: u32,
    pub possible: bool,
}

// https://docs.rs/bitcoincore-rpc-json/0.12.0/src/bitcoincore_rpc_json/lib.rs.html#569-577
#[derive(Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Bip9SoftforkInfo {
    pub status: Bip9SoftforkStatus,
    pub bit: Option<u8>,
    // Can be -1 for 0.18.x inactive ones.
    pub start_time: i64,
    pub timeout: u64,
    pub since: u32,
    pub statistics: Option<Bip9SoftforkStatistics>,
}

// https://docs.rs/bitcoincore-rpc-json/0.12.0/src/bitcoincore_rpc_json/lib.rs.html#581-584
#[derive(Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SoftforkType {
    Buried,
    Bip9,
}

// https://docs.rs/bitcoincore-rpc-json/0.12.0/src/bitcoincore_rpc_json/lib.rs.html#588-594
#[derive(Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Softfork {
    #[serde(rename = "type")]
    pub type_: SoftforkType,
    pub bip9: Option<Bip9SoftforkInfo>,
    pub height: Option<u32>,
    pub active: bool,
}

// https://docs.rs/bitcoincore-rpc-json/0.12.0/src/bitcoincore_rpc_json/lib.rs.html#700-740
#[derive(Debug, Deserialize, Serialize)]
pub struct BlockchainInfo {
    /// Current network name as defined in BIP70 (main, test, regtest)
    pub chain: String,
    /// The current number of blocks processed in the server
    pub blocks: u64,
    /// The current number of headers we have validated
    pub headers: u64,
    /// The hash of the currently best block
    pub bestblockhash: bitcoin::BlockHash,
    /// The current difficulty
    pub difficulty: f64,
    /// Median time for the current best block
    pub mediantime: u64,
    /// Estimate of verification progress [0..1]
    pub verificationprogress: f64,
    /// Estimate of whether this node is in Initial Block Download mode
    pub initialblockdownload: bool,
    /// Total amount of work in active chain, in hexadecimal
    pub chainwork: HexBytes,
    /// The estimated size of the block and undo files on disk
    pub size_on_disk: u64,
    /// If the blocks are subject to pruning
    pub pruned: bool,
    /// Lowest-height complete block stored (only present if pruning is enabled)
    pub pruneheight: Option<u64>,
    /// Whether automatic pruning is enabled (only present if pruning is enabled)
    pub automatic_pruning: Option<bool>,
    /// The target size used by pruning (only present if automatic pruning is enabled)
    pub prune_target_size: Option<u64>,
    /// Status of softforks in progress
    #[serde(default)]
    pub softforks: LinearMap<String, Softfork>,
    /// Any network and blockchain warnings.
    pub warnings: String,
}

impl RpcMethod for GetBlockchainInfo {
    type Params = [(); 0];
    type Response = BlockchainInfo;
    fn as_str(&self) -> &'static str {
        "getblockchaininfo"
    }
}
impl Serialize for GetBlockchainInfo {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.as_str().serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for GetBlockchainInfo {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s: &'de str = Deserialize::deserialize(deserializer)?;
        if s == Self.as_str() {
            Ok(Self)
        } else {
            Err(serde::de::Error::invalid_value(
                serde::de::Unexpected::Str(s),
                &Self.as_str(),
            ))
        }
    }
}

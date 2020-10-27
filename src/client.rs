use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::PathBuf;

use anyhow::{anyhow, Error};
use bitcoin::{
    consensus::Decodable,
    hash_types::BlockHash,
    network::{constants::ServiceFlags, Address},
    util::amount::Amount,
};
use futures::{channel::mpsc, StreamExt, TryStreamExt};
use hyper::{
    body::Bytes,
    client::{Client, HttpConnector},
    header::{HeaderValue, AUTHORIZATION, CONTENT_LENGTH},
    Body, Method, Request, Response, StatusCode, Uri,
};
use itertools::Itertools;
use serde::{
    de::{Deserialize, Deserializer},
    ser::{Serialize, Serializer},
};
use serde_json::Value;

use crate::util::{Either, HexBytes};

#[cfg(feature = "compat")]
use crate::util::compat::StrCompat;

pub const MISC_ERROR_CODE: i64 = -1;
pub const METHOD_NOT_ALLOWED_ERROR_CODE: i64 = -32604;
pub const PARSE_ERROR_CODE: i64 = -32700;
pub const METHOD_NOT_ALLOWED_ERROR_MESSAGE: &'static str = "Method not allowed";
pub const PRUNE_ERROR_MESSAGE: &'static str = "Block not available (pruned data)";

type HttpClient = Client<HttpConnector>;

#[derive(Debug)]
pub enum SingleOrBatchRpcRequest {
    Single(RpcRequest<GenericRpcMethod>),
    Batch(Vec<RpcRequest<GenericRpcMethod>>),
}
impl Serialize for SingleOrBatchRpcRequest {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            SingleOrBatchRpcRequest::Single(s) => s.serialize(serializer),
            SingleOrBatchRpcRequest::Batch(b) => b.serialize(serializer),
        }
    }
}
impl<'de> Deserialize<'de> for SingleOrBatchRpcRequest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = SingleOrBatchRpcRequest;
            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                write!(
                    formatter,
                    "a single rpc request, or a batch of rpc requests"
                )
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<Self::Value, A::Error> {
                let mut res = Vec::new();
                while let Some(elem) = seq.next_element()? {
                    res.push(elem);
                }
                Ok(SingleOrBatchRpcRequest::Batch(res))
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                let mut id = None;
                let mut method = None;
                let mut params = None;
                while let Some(key) = map.next_key()? {
                    match key {
                        "id" => {
                            id = map.next_value()?;
                        }
                        "method" => {
                            method = map.next_value()?;
                        }
                        "params" => {
                            params = map.next_value()?;
                        }
                        _ => {
                            let _: serde_json::Value = map.next_value()?;
                        }
                    }
                }
                Ok(SingleOrBatchRpcRequest::Single(RpcRequest {
                    id,
                    method: method.ok_or_else(|| serde::de::Error::missing_field("method"))?,
                    params: params.ok_or_else(|| serde::de::Error::missing_field("params"))?,
                }))
            }
        }
        deserializer.deserialize_any(Visitor)
    }
}

pub trait RpcMethod {
    type Params: Serialize + for<'de> Deserialize<'de>;
    type Response: Serialize + for<'de> Deserialize<'de>;
    fn as_str<'a>(&'a self) -> &'a str;
}

#[derive(Debug, serde::Serialize, serde::Deserialize, Deref)]
pub struct GenericRpcMethod(pub String);
impl RpcMethod for GenericRpcMethod {
    type Params = Vec<Value>;
    type Response = Value;
    fn as_str<'a>(&'a self) -> &'a str {
        self.0.as_str()
    }
}

#[derive(Debug)]
pub struct GetBlock;
#[derive(Debug, serde::Serialize, serde::Deserialize)]
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
#[derive(Debug, serde::Serialize, serde::Deserialize)]
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
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct PeerInfo {
    /// Peer index
    pub id: u64,
    /// The IP address and port of the peer
    pub addr: String,
    /// Bind address of the connection to the peer
    // TODO: use a type for addrbind
    pub addrbind: String,
    /// Local address as reported by the peer
    // TODO: use a type for addrlocal
    pub addrlocal: Option<String>,
    /// The services offered
    // TODO: use a type for services
    pub services: String,
    /// The services offered
    pub servicesnames: HashSet<String>,
    /// Whether peer has asked us to relay transactions to it
    pub relaytxes: bool,
    /// The time in seconds since epoch (Jan 1 1970 GMT) of the last send
    pub lastsend: u64,
    /// The time in seconds since epoch (Jan 1 1970 GMT) of the last receive
    pub lastrecv: u64,
    /// The total bytes sent
    pub bytessent: u64,
    /// The total bytes received
    pub bytesrecv: u64,
    /// The connection time in seconds since epoch (Jan 1 1970 GMT)
    pub conntime: u64,
    /// The time offset in seconds
    pub timeoffset: i64,
    /// ping time (if available)
    pub pingtime: Option<f64>,
    /// minimum observed ping time (if any at all)
    pub minping: Option<f64>,
    /// ping wait (if non-zero)
    pub pingwait: Option<f64>,
    /// The peer version, such as 70001
    pub version: u64,
    /// The string version
    pub subver: String,
    /// Inbound (true) or Outbound (false)
    pub inbound: bool,
    /// Whether connection was due to `addnode`/`-connect` or if it was an
    /// automatic/inbound connection
    pub addnode: bool,
    /// The starting height (block) of the peer
    pub startingheight: i64,
    /// The ban score
    pub banscore: i64,
    /// The last header we have in common with this peer
    pub synced_headers: i64,
    /// The last block we have in common with this peer
    pub synced_blocks: i64,
    /// The heights of blocks we're currently asking from this peer
    pub inflight: Vec<u64>,
    /// Whether the peer is whitelisted
    pub whitelisted: bool,
    #[serde(
        rename = "minfeefilter",
        default,
        with = "bitcoin::util::amount::serde::as_btc::opt"
    )]
    pub min_fee_filter: Option<Amount>,
    /// The total bytes sent aggregated by message type
    pub bytessent_per_msg: HashMap<String, u64>,
    /// The total bytes received aggregated by message type
    pub bytesrecv_per_msg: HashMap<String, u64>,
}
impl PeerInfo {
    pub fn into_address(self) -> Result<Address, Error> {
        let services = ServiceFlags::consensus_decode(&mut std::io::Cursor::new(hex::decode(
            &self.services,
        )?))?;
        if let Ok(sock_addr) = self.addr.parse() {
            Ok(Address::new(&sock_addr, services))
        } else {
            let mut addr_split = self.addr.split(":");
            let host = addr_split
                .next()
                .ok_or_else(|| anyhow!("Invalid Peer Address: {}", self.addr))?;
            let port = addr_split
                .next()
                .ok_or_else(|| anyhow!("Invalid Peer Address: {}", self.addr))?
                .parse()?;
            let onion = base32::decode(
                base32::Alphabet::RFC4648 { padding: false },
                host.strip_suffix(".onion")
                    .ok_or_else(|| anyhow!("Invalid Peer Address: {}", self.addr))?,
            )
            .ok_or_else(|| anyhow!("Invalid Peer Address: {}", self.addr))?;
            if onion.len() < 10 {
                return Err(anyhow!("Invalid Peer Address: {}", self.addr));
            }
            let address: [u16; 8] = [
                0xFD87,
                0xD87E,
                0xEB43,
                ((onion[0] as u16) << 8) + (onion[1] as u16),
                ((onion[2] as u16) << 8) + (onion[3] as u16),
                ((onion[4] as u16) << 8) + (onion[5] as u16),
                ((onion[6] as u16) << 8) + (onion[7] as u16),
                ((onion[8] as u16) << 8) + (onion[9] as u16),
            ];
            Ok(Address {
                services,
                address,
                port,
            })
        }
    }
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

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct RpcRequest<T: RpcMethod> {
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    pub method: T,
    pub params: T::Params,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip)]
    pub status: Option<StatusCode>,
}
impl From<Error> for RpcError {
    fn from(e: Error) -> Self {
        RpcError {
            code: MISC_ERROR_CODE,
            message: format!("{}", e),
            status: None,
        }
    }
}
impl From<serde_json::Error> for RpcError {
    fn from(e: serde_json::Error) -> Self {
        RpcError {
            code: PARSE_ERROR_CODE,
            message: format!("{}", e),
            status: None,
        }
    }
}
impl From<RpcError> for Error {
    fn from(e: RpcError) -> Self {
        anyhow!("{}", e.message)
    }
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct RpcResponse<T: RpcMethod> {
    pub id: Option<Value>,
    pub error: Option<RpcError>,
    pub result: Option<T::Response>,
}
impl From<RpcError> for RpcResponse<GenericRpcMethod> {
    fn from(e: RpcError) -> Self {
        RpcResponse {
            id: None,
            error: Some(e),
            result: None,
        }
    }
}
impl<T: RpcMethod> RpcResponse<T> {
    pub fn into_result(self) -> Result<T::Response, RpcError> {
        match self.error {
            Some(e) => Err(e),
            None => Ok(self.result).transpose().unwrap_or_else(|| {
                serde_json::from_value(Value::Null)
                    .map_err(Error::from)
                    .map_err(RpcError::from)
            }),
        }
    }
    pub fn into_response(mut self) -> Result<Response<Body>, Error> {
        let body = serde_json::to_vec(&self)?;
        Ok(Response::builder()
            .status(match self.error.as_mut().and_then(|e| e.status.take()) {
                Some(s) => s,
                None if self.error.is_some() => StatusCode::INTERNAL_SERVER_ERROR,
                None => StatusCode::OK,
            })
            .header(CONTENT_LENGTH, body.len())
            .body(body.into())?)
    }
}

#[derive(Debug)]
pub struct RpcClient {
    authorization: HeaderValue,
    uri: Uri,
    client: HttpClient,
}
impl RpcClient {
    pub fn new(auth: AuthSource, uri: Uri) -> Result<Self, Error> {
        Ok(RpcClient {
            authorization: auth.try_load()?,
            uri,
            client: HttpClient::new(),
        })
    }
    pub async fn send<
        'a,
        F: Fn(&'a RpcRequest<GenericRpcMethod>) -> Fut,
        Fut: Future<Output = Result<Option<RpcResponse<GenericRpcMethod>>, RpcError>> + 'a,
    >(
        &self,
        req: &'a SingleOrBatchRpcRequest,
        intercept: F,
    ) -> Result<Response<Body>, Error> {
        match req {
            SingleOrBatchRpcRequest::Single(req) => {
                Ok(if let Some(res) = intercept(req).await.transpose() {
                    res.unwrap_or_else(|e| RpcResponse {
                        id: req.id.clone(),
                        result: None,
                        error: Some(e),
                    })
                    .into_response()?
                } else {
                    self.client
                        .request(
                            Request::builder()
                                .method(Method::POST)
                                .header(AUTHORIZATION, &self.authorization)
                                .uri(&self.uri)
                                .body(serde_json::to_string(req)?.into())?,
                        )
                        .await?
                })
            }
            SingleOrBatchRpcRequest::Batch(reqs) => {
                let (intercepted_send, intercepted_recv) = mpsc::unbounded();
                let (forwarded_send, forwarded_recv) = mpsc::unbounded();
                let intercept_fn = &intercept;
                futures::stream::iter(reqs.iter().enumerate())
                    .for_each_concurrent(None, move |(idx, req)| {
                        let intercepted_send = intercepted_send.clone();
                        let forwarded_send = forwarded_send.clone();
                        async move {
                            match intercept_fn(req).await.transpose() {
                                Some(res) => intercepted_send
                                    .unbounded_send(res.map(|res| (idx, res)))
                                    .unwrap(),
                                None => forwarded_send.unbounded_send((idx, req)).unwrap(),
                            }
                        }
                    })
                    .await;
                async fn send_batch(
                    client: &RpcClient,
                    forwarded_recv: mpsc::UnboundedReceiver<(usize, &RpcRequest<GenericRpcMethod>)>,
                ) -> Result<Vec<(usize, RpcResponse<GenericRpcMethod>)>, RpcError> {
                    let (idxs, new_batch): (Vec<usize>, Vec<_>) =
                        forwarded_recv.collect::<Vec<_>>().await.into_iter().unzip();
                    let response = client
                        .client
                        .request(
                            Request::builder()
                                .method(Method::POST)
                                .header(AUTHORIZATION, &client.authorization)
                                .uri(&client.uri)
                                .body(serde_json::to_string(&new_batch)?.into())
                                .map_err(Error::from)?,
                        )
                        .await
                        .map_err(Error::from)?;
                    let body: Bytes =
                        tokio::stream::StreamExt::collect::<Result<Bytes, _>>(response.into_body())
                            .await
                            .map_err(Error::from)?;
                    let forwarded_res: Vec<RpcResponse<GenericRpcMethod>> =
                        serde_json::from_slice(body.as_ref())?;
                    Ok(idxs.into_iter().zip(forwarded_res).collect())
                }
                let (forwarded, intercepted) = match futures::try_join!(
                    send_batch(self, forwarded_recv),
                    intercepted_recv.try_collect::<Vec<_>>()
                ) {
                    Ok(a) => a,
                    Err(e) => return Ok(RpcResponse::from(e).into_response()?),
                };
                let res_vec: Vec<RpcResponse<GenericRpcMethod>> = forwarded
                    .into_iter()
                    .merge_by(intercepted, |(a, _), (b, _)| a < b)
                    .map(|(_, res)| res)
                    .collect();
                let body = serde_json::to_vec(&res_vec)?;
                Ok(Response::builder()
                    .header(CONTENT_LENGTH, body.len())
                    .body(body.into())?)
            }
        }
    }
    pub async fn call<T: RpcMethod + Serialize>(
        &self,
        req: &RpcRequest<T>,
    ) -> Result<RpcResponse<T>, Error> {
        let response = self
            .client
            .request(
                Request::builder()
                    .method(Method::POST)
                    .header(AUTHORIZATION, &self.authorization)
                    .uri(&self.uri)
                    .body(serde_json::to_string(req)?.into())?,
            )
            .await?;
        let status = response.status();
        let body: Bytes =
            tokio::stream::StreamExt::collect::<Result<Bytes, _>>(response.into_body()).await?;
        let mut rpc_response: RpcResponse<T> = serde_json::from_slice(&body)?;
        if let Some(ref mut error) = rpc_response.error {
            error.status = Some(status);
        }
        Ok(rpc_response)
    }
}

pub enum AuthSource {
    Const { username: String, password: String },
    CookieFile(PathBuf),
}

impl AuthSource {
    pub fn from_config(
        user: Option<String>,
        password: Option<String>,
        file: Option<PathBuf>,
    ) -> Result<Self, Error> {
        match (user, password, file) {
            (Some(username), Some(password), None) => Ok(AuthSource::Const { username, password }),
            (None, None, Some(cookie_file)) => Ok(AuthSource::CookieFile(cookie_file)),
            // It could pull it from bitcoin.conf, but I don't think it's worth my time.
            // PRs open.
            (None, None, None) => Err(anyhow!("missing authentication information")),
            _ => Err(anyhow!(
                "either a password and possibly a username or a cookie file must be specified"
            )),
        }
    }

    fn load_from_file(path: &PathBuf) -> Result<String, Error> {
        Ok(std::fs::read_to_string(path).map(|mut cookie| {
            if cookie.ends_with('\n') {
                cookie.pop();
            }
            cookie
        })?)
    }

    pub fn try_load(&self) -> Result<HeaderValue, Error> {
        Ok(format!(
            "Basic {}",
            match self {
                AuthSource::Const { username, password } =>
                    base64::encode(format!("{}:{}", username, password)),
                AuthSource::CookieFile(path) => AuthSource::load_from_file(path)?,
            }
        )
        .parse()?)
    }
}

use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use anyhow::{anyhow, Error};
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
use tokio::sync::RwLock;

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
                let mut res = Vec::with_capacity(seq.size_hint().unwrap_or(16));
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

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct GenericRpcMethod(pub String);

impl RpcMethod for GenericRpcMethod {
    type Params = Vec<Value>;
    type Response = Value;
    fn as_str<'a>(&'a self) -> &'a str {
        self.0.as_str()
    }
}

impl std::ops::Deref for GenericRpcMethod {
    type Target = String;

    fn deref(&self) -> &Self::Target {
        &self.0
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

#[derive(Debug, thiserror::Error, serde::Serialize, serde::Deserialize)]
#[error("bitcoin RPC failed with code {code}, message: {message}")]
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
    authorization: AuthSource,
    uri: Uri,
    client: HttpClient,
    logger: slog::Logger,
}
impl RpcClient {
    pub fn new(auth: AuthSource, uri: Uri, logger: &slog::Logger) -> Self {
        let uri_string = uri.to_string();
        RpcClient {
            authorization: auth, // DO NOT try to eager evaluate this, it can change while the program is running
            uri,
            client: HttpClient::new(),
            logger: logger.new(o!("uri" => uri_string)),
        }
    }
    pub async fn send<
        'a,
        F: Fn(&'a str, &'a RpcRequest<GenericRpcMethod>) -> Fut,
        Fut: Future<Output = Result<Option<RpcResponse<GenericRpcMethod>>, RpcError>> + 'a,
    >(
        &self,
        path: &'a str,
        req: &'a SingleOrBatchRpcRequest,
        intercept: F,
    ) -> Result<Response<Body>, Error> {
        match req {
            SingleOrBatchRpcRequest::Single(req) => {
                Ok(if let Some(res) = intercept(path, req).await.transpose() {
                    res.unwrap_or_else(|e| RpcResponse {
                        id: req.id.clone(),
                        result: None,
                        error: Some(e),
                    })
                    .into_response()?
                } else {
                    let mut parts = self.uri.clone().into_parts();
                    parts.path_and_query = Some(path.parse()?);
                    self.client
                        .request(
                            Request::builder()
                                .method(Method::POST)
                                .header(AUTHORIZATION, self.authorization.try_load().await?)
                                .uri(Uri::from_parts(parts)?)
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
                            match intercept_fn(path, req).await.transpose() {
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
                    path: &str,
                    forwarded_recv: mpsc::UnboundedReceiver<(usize, &RpcRequest<GenericRpcMethod>)>,
                ) -> Result<Vec<(usize, RpcResponse<GenericRpcMethod>)>, RpcError> {
                    let (idxs, new_batch): (Vec<usize>, Vec<_>) =
                        forwarded_recv.collect::<Vec<_>>().await.into_iter().unzip();
                    let mut parts = client.uri.clone().into_parts();
                    parts.path_and_query = Some(path.parse().map_err(Error::from)?);
                    let authorization = match client.authorization.try_load().await {
                        Ok(authorization) => authorization,
                        Err(error) => {
                            error!(client.logger, "Failed to load authorization"; "error" => #error);
                            // We need to explicitly turn this error into internal server error to
                            // not leak information
                            return Err(RpcError {
                                code: MISC_ERROR_CODE,
                                message: "internal server error".to_owned(),
                                status: Some(StatusCode::INTERNAL_SERVER_ERROR),
                            });
                        },
                    };

                    let response = client
                        .client
                        .request(
                            Request::builder()
                                .method(Method::POST)
                                .header(AUTHORIZATION, authorization)
                                .uri(Uri::from_parts(parts).map_err(Error::from)?)
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
                    send_batch(self, path, forwarded_recv),
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
    ) -> Result<RpcResponse<T>, ClientError> {
        let response = self
            .client
            .request(
                Request::builder()
                    .method(Method::POST)
                    .header(AUTHORIZATION, self.authorization.try_load().await?)
                    .uri(&self.uri)
                    .body(serde_json::to_string(req)?.into())?,
            )
            .await?;
        let status = response.status();
        let body: Bytes =
            tokio::stream::StreamExt::collect::<Result<Bytes, _>>(response.into_body()).await?;
        let mut rpc_response: RpcResponse<T> = serde_json::from_slice(&body)
            .map_err(|serde_error| {
                match std::str::from_utf8(&body) {
                    Ok(body) => ClientError::ParseResponseUtf8 { method: req.method.as_str().to_owned(), status: status, body: body.to_owned(), serde_error },
                    Err(error) => ClientError::ResponseNotUtf8 { method: req.method.as_str().to_owned(), status: status, utf8_error: error, },
                }
            })?;
        if let Some(ref mut error) = rpc_response.error {
            error.status = Some(status);
        }
        Ok(rpc_response)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("serialization failed")]
    Serde(#[from] serde_json::Error),
    #[error("failed to load authentication data")]
    LoadAuth(#[from] AuthLoadError),
    #[error("hyper failed to process HTTP request")]
    Hyper(#[from] hyper::error::Error),
    #[error("invalid HTTP request")]
    Http(#[from] http::Error),
    #[error("HTTP response (status: {status}) to method {method} can't be parsed as json, body: {body}")]
    ParseResponseUtf8 { method: String, status: http::status::StatusCode, body: String, #[source] serde_error: serde_json::Error },
    #[error("HTTP response (status: {status}) to method {method} is not UTF-8")]
    ResponseNotUtf8 { method: String, status: http::status::StatusCode, utf8_error: std::str::Utf8Error, },
}

impl From<ClientError> for RpcError {
    fn from(error: ClientError) -> Self {
        RpcError {
            code: MISC_ERROR_CODE,
            message: error.to_string(),
            status: None,
        }
    }
}

#[derive(Debug)]
pub enum AuthSource {
    Const {
        username: String,
        password: String,
        header: HeaderValue,
    },
    CookieFile {
        path: PathBuf,
        cached: RwLock<Option<Arc<(SystemTime, HeaderValue)>>>,
    },
}

impl AuthSource {
    pub fn from_config(
        user: Option<String>,
        password: Option<String>,
        file: Option<PathBuf>,
    ) -> Result<Self, Error> {
        match (user, password, file) {
            (Some(username), Some(password), None) => Ok(AuthSource::Const {
                header: format!(
                    "Basic {}",
                    base64::encode(format!("{}:{}", username, password))
                )
                .parse()?,
                username,
                password,
            }),
            (None, None, Some(cookie_file)) => Ok(AuthSource::CookieFile {
                path: cookie_file,
                cached: RwLock::new(None),
            }),
            // It could pull it from bitcoin.conf, but I don't think it's worth my time.
            // PRs open.
            (None, None, None) => Err(anyhow!("missing authentication information")),
            _ => Err(anyhow!(
                "either a password and possibly a username or a cookie file must be specified"
            )),
        }
    }

    async fn load_from_file(path: &PathBuf) -> Result<String, AuthLoadError> {
        tokio::fs::read_to_string(path).await.map(|mut cookie| {
            if cookie.ends_with('\n') {
                cookie.pop();
            }
            base64::encode(cookie)
        })
        .map_err(|error| AuthLoadError::Read { path: path.to_owned(), error, })
    }

    pub async fn try_load(&self) -> Result<HeaderValue, AuthLoadError> {
        match self {
            AuthSource::Const { ref header, .. } => Ok(header.clone()),
            AuthSource::CookieFile {
                ref path,
                ref cached,
            } => {
                let cache = cached.read().await.clone();
                let modified = tokio::fs::metadata(&path)
                    .await
                    .map_err(|error| AuthLoadError::Metadata { path: path.to_owned(), error, })?
                    .modified()
                    .map_err(|error| AuthLoadError::Modified { path: path.to_owned(), error, })?;
                match cache {
                    Some(cache) if modified == cache.0 => Ok(cache.1.clone()),
                    _ => {
                        let header: HeaderValue =
                            format!("Basic {}", AuthSource::load_from_file(path).await?).parse()?;
                        let new_cache = (modified, header.clone());
                        *cached.write().await = Some(Arc::new(new_cache));
                        Ok(header)
                    }
                }
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AuthLoadError {
    #[error("failed to get metadata of file {path}")]
    Metadata { path: PathBuf, #[source] error: std::io::Error, },
    #[error("failed to get modification time of file {path}")]
    Modified { path: PathBuf, #[source] error: std::io::Error, },
    #[error("failed to read file {path}")]
    Read { path: PathBuf, #[source] error: std::io::Error, },
    #[error("invalid header value")]
    HeaderValue(#[from] http::header::InvalidHeaderValue),
}

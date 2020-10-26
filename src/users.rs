use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::Error;
use bitcoin::consensus::Encodable;
use hyper::{header::HeaderValue, StatusCode};
use serde_json::Value;

use crate::client::{
    GenericRpcMethod, GetBlock, GetBlockHeader, GetBlockHeaderParams, GetBlockResult, RpcError,
    RpcMethod, RpcRequest, RpcResponse, METHOD_NOT_ALLOWED_ERROR_CODE,
    METHOD_NOT_ALLOWED_ERROR_MESSAGE, MISC_ERROR_CODE, PRUNE_ERROR_MESSAGE,
};
use crate::env::Env;
use crate::fetch_blocks::fetch_block;

#[cfg(feature = "compat")]
use crate::util::compat::StrCompat;

#[derive(Debug, serde::Deserialize)]
pub struct Users(pub HashMap<String, User>);
impl Users {
    pub fn get(&self, auth: &HeaderValue) -> Option<&User> {
        let header_str = auth.to_str().ok()?;
        let auth = header_str.strip_prefix("Basic ")?;
        let auth_decoded = base64::decode(auth).ok()?;
        let auth_decoded_str = std::str::from_utf8(&auth_decoded).ok()?;
        let mut auth_split = auth_decoded_str.split(":");
        let name = auth_split.next()?;
        let pass = auth_split.next()?;
        self.0.get(name).filter(|u| u.password == pass)
    }
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct User {
    pub password: String,
    pub allowed_calls: HashSet<String>,
    pub fetch_blocks: bool,
}
impl User {
    pub async fn intercept<'a>(
        &self,
        env: Arc<Env>,
        req: &'a RpcRequest<GenericRpcMethod>,
    ) -> Result<Option<RpcResponse<GenericRpcMethod>>, RpcError> {
        if self.allowed_calls.contains(&*req.method) {
            if self.fetch_blocks && &*req.method == GetBlock.as_str()
            // only non-verbose for now
            {
                match req.params.get(1).unwrap_or(&1_u64.into()) {
                    Value::Number(ref n) if n.as_u64() == Some(0) => {
                        match fetch_block(
                            env.clone(),
                            env.get_peers().await?,
                            serde_json::from_value(req.params[0].clone()).map_err(Error::from)?,
                        )
                        .await
                        {
                            Ok(Some(block)) => {
                                let mut block_data = Vec::new();
                                block
                                    .consensus_encode(&mut block_data)
                                    .map_err(Error::from)?;
                                let block_data = hex::encode(&block_data);
                                Ok(Some(RpcResponse {
                                    id: req.id.clone(),
                                    result: Some(Value::String(block_data)),
                                    error: None,
                                }))
                            }
                            Ok(None) => Ok(Some(RpcResponse {
                                id: req.id.clone(),
                                result: None,
                                error: Some(RpcError {
                                    code: MISC_ERROR_CODE,
                                    message: PRUNE_ERROR_MESSAGE.to_owned(),
                                    status: None,
                                }),
                            })),
                            Err(e) => Ok(Some(e.into())),
                        }
                    }
                    Value::Number(ref n) if n.as_u64() == Some(1) => {
                        let hash =
                            serde_json::from_value(req.params[0].clone()).map_err(Error::from)?;
                        let fetch_header_req = RpcRequest {
                            id: None,
                            method: GetBlockHeader,
                            params: GetBlockHeaderParams(hash, Some(true)),
                        };
                        match futures::try_join!(
                            async { env.rpc_client.call(&fetch_header_req).await?.into_result() },
                            async { fetch_block(env.clone(), env.get_peers().await?, hash).await }
                        ) {
                            Ok((header, Some(block))) => Ok(Some(RpcResponse {
                                id: req.id.clone(),
                                result: {
                                    let size = block.get_size();
                                    let witness = block
                                        .txdata
                                        .iter()
                                        .flat_map(|tx| tx.input.iter())
                                        .flat_map(|input| input.witness.iter())
                                        .map(|witness| witness.len())
                                        .fold(0, |acc, x| acc + x);
                                    Some(serde_json::to_value(GetBlockResult {
                                        header: header.into_right().ok_or_else(|| {
                                            anyhow::anyhow!(
                                                "unexpected response for getblockheader"
                                            )
                                        })?,
                                        size,
                                        strippedsize: if witness > 0 {
                                            Some(size - witness)
                                        } else {
                                            None
                                        },
                                        weight: block.get_weight(),
                                        tx: block.txdata.into_iter().map(|tx| tx.txid()).collect(),
                                    })?)
                                },
                                error: None,
                            })),
                            Ok((_, None)) => Ok(Some(RpcResponse {
                                id: req.id.clone(),
                                result: None,
                                error: Some(RpcError {
                                    code: MISC_ERROR_CODE,
                                    message: PRUNE_ERROR_MESSAGE.to_owned(),
                                    status: None,
                                }),
                            })),
                            Err(e) => Ok(Some(e.into())),
                        }
                    }
                    _ => Ok(None), // TODO
                }
            } else {
                Ok(None)
            }
        } else {
            Err(RpcError {
                code: METHOD_NOT_ALLOWED_ERROR_CODE,
                message: METHOD_NOT_ALLOWED_ERROR_MESSAGE.to_owned(),
                status: Some(StatusCode::FORBIDDEN),
            })
        }
    }
}

use std::collections::{HashMap, HashSet};

use anyhow::Error;
use bitcoin::consensus::Encodable;
use hyper::{header::HeaderValue, StatusCode};
use serde_json::Value;

use crate::client::{
    GenericRpcMethod, GetBlock, RpcError, RpcMethod, RpcRequest, RpcResponse,
    METHOD_NOT_ALLOWED_ERROR_CODE, METHOD_NOT_ALLOWED_ERROR_MESSAGE, MISC_ERROR_CODE,
    PRUNE_ERROR_MESSAGE,
};
use crate::env::Env;
use crate::fetch_blocks::fetch_block;

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
pub struct User {
    pub password: String,
    pub allowed_calls: HashSet<String>,
    pub fetch_blocks: bool,
}
impl User {
    pub async fn intercept<'a>(
        &'static self,
        env: &'static Env,
        req: &'a RpcRequest<GenericRpcMethod>,
    ) -> Option<RpcResponse<GenericRpcMethod>> {
        async fn intercept_res<'a>(
            user: &'static User,
            env: &'static Env,
            req: &'a RpcRequest<GenericRpcMethod>,
        ) -> Result<Option<RpcResponse<GenericRpcMethod>>, RpcError> {
            if user.allowed_calls.contains(&*req.method) {
                if user.fetch_blocks
                    && &*req.method == GetBlock.as_str()
                    && req.params.get(1) == Some(&Value::Number(0.into()))
                // only non-verbose for now
                {
                    match fetch_block(
                        env,
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
                        Ok(None) => Err(RpcError {
                            code: MISC_ERROR_CODE,
                            message: PRUNE_ERROR_MESSAGE.to_owned(),
                            status: None,
                        }),
                        Err(e) => Ok(Some(e.into())),
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
        match intercept_res(self, env, req).await {
            Ok(a) => a,
            Err(e) => Some(RpcResponse {
                id: req.id.clone(),
                result: None,
                error: Some(e),
            }),
        }
    }
}

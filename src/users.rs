use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::Error;
use bitcoin::consensus::Encodable;
use hyper::{header::HeaderValue, StatusCode};
use serde_json::Value;

use crate::client::{
    GenericRpcMethod, RpcError, RpcMethod, RpcRequest, RpcResponse, METHOD_NOT_ALLOWED_ERROR_CODE,
    METHOD_NOT_ALLOWED_ERROR_MESSAGE, MISC_ERROR_CODE, PRUNE_ERROR_MESSAGE,
};
use crate::fetch_blocks::fetch_block;
use crate::rpc_methods::{
    GetBlock, GetBlockHeader, GetBlockHeaderParams, GetBlockResult, GetBlockchainInfo,
};
use crate::state::State;

#[cfg(feature = "old_rust")]
use crate::util::old_rust::StrCompat;

pub use password::Password;

pub mod input {
    use std::collections::{HashMap, HashSet};

    #[derive(Debug, serde::Deserialize)]
    pub struct User {
        pub password: super::Password,
        pub allowed_calls: HashSet<String>,
        #[serde(default)]
        pub fetch_blocks: Option<bool>,
        #[serde(default)]
        pub override_wallet: Option<String>,
    }

    impl User {
        fn map_default(self, default_fetch_blocks: bool) -> super::User {
            let wallet = self.override_wallet.map(|mut wallet| {
                wallet.insert_str(0, "/wallet/");
                wallet
            });
            super::User {
                password: self.password,
                allowed_calls: self.allowed_calls,
                fetch_blocks: self.fetch_blocks.unwrap_or(default_fetch_blocks),
                override_wallet: wallet,
            }
        }
    }

    pub fn map_default(users: HashMap<String, User>, default_fetch_blocks: bool) -> super::Users {
        super::Users(users.into_iter().map(|(name, user)| (name, user.map_default(default_fetch_blocks))).collect())
    }
}

mod password {
    use std::fmt;
    use std::convert::TryFrom;
    use std::ffi::{OsStr, OsString};

    #[derive(serde::Deserialize)]
    #[serde(try_from = "String")]
    pub struct Password(String);

    impl Password {
        fn validate_str(string: &str) -> Result<(), InvalidPasswordError> {
            for (pos, byte) in string.bytes().enumerate() {
                if byte <= 0x1F || byte >= 0x7F {
                    return Err(InvalidPasswordError(InvalidPasswordErrorInner::BadChar { pos, byte, }))
                }
            }
            Ok(())
        }
    }

    impl fmt::Debug for Password {
        fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("Password(secret)")
        }
    }

    impl TryFrom<String> for Password {
        type Error = InvalidPasswordError;

        fn try_from(string: String) -> Result<Self, Self::Error> {
            Password::validate_str(&string)?;
            Ok(Password(string))
        }
    }

    impl TryFrom<&'_ str> for Password {
        type Error = InvalidPasswordError;

        fn try_from(string: &str) -> Result<Self, Self::Error> {
            Password::validate_str(string)?;
            Ok(Password(string.to_owned()))
        }
    }

    impl configure_me::parse_arg::ParseArg for Password {
        type Error = InvalidPasswordError;

        fn parse_arg(arg: &OsStr) -> Result<Self, Self::Error> {
            let string = arg.to_str().ok_or(InvalidPasswordError(InvalidPasswordErrorInner::NonAscii))?;
            Password::validate_str(string)?;
            Ok(Password(string.to_owned()))
        }

        fn parse_owned_arg(arg: OsString) -> Result<Self, Self::Error> {
            let string = arg.into_string().map_err(|_| InvalidPasswordError(InvalidPasswordErrorInner::NonAscii))?;
            Password::validate_str(&string)?;
            Ok(Password(string.to_owned()))
        }

        fn describe_type<W: fmt::Write>(mut writer: W) -> fmt::Result {
            writer.write_str("an ASCII string with no control characters")
        }
    }

    impl PartialEq<&'_ str> for Password {
        fn eq(&self, other: &&str) -> bool {
            // timing safe equality
            #[inline(never)]
            fn xor_contents(a: &[u8], b: &[u8]) -> usize {
                a
                    .iter()
                    .enumerate()
                    .map(|(i, byte)| *byte ^ b[i % b.len()])
                    .fold(a.len() ^ b.len(), |acc, item| acc | usize::from(item))
            }

            if self.0.is_empty() {
                return other.is_empty()
            }

            let bits = xor_contents(self.0.as_bytes(), other.as_bytes());
            unsafe { std::ptr::read_volatile(&bits) == 0 }
        }
    }

    impl PartialEq for Password {
        fn eq(&self, other: &Password) -> bool {
            *self == &*other.0
        }
    }

    #[derive(Debug, thiserror::Error)]
    enum InvalidPasswordErrorInner {
        #[error("invalid byte 0x{byte:02X} at position {pos}")]
        BadChar { pos: usize, byte: u8, },
        // non-utf-8 implies non-ascii
        #[error("not an ascii string")]
        NonAscii,
    }

    #[derive(Debug, thiserror::Error)]
    #[error(transparent)]
    pub struct InvalidPasswordError(InvalidPasswordErrorInner);
}

#[derive(Debug, serde::Deserialize)]
pub struct Users(pub HashMap<String, User>);
impl Users {
    pub fn get(&self, auth: &HeaderValue) -> Option<(String, &User)> {
        let header_str = auth.to_str().ok()?;
        let auth = header_str.strip_prefix("Basic ")?;
        let auth_decoded = base64::decode(auth).ok()?;
        let auth_decoded_str = std::str::from_utf8(&auth_decoded).ok()?;
        let mut auth_split = auth_decoded_str.split(":");
        let name = auth_split.next()?;
        let pass = auth_split.next()?;
        self.0
            .get(name)
            .filter(|u| u.password == pass)
            .map(|u| (name.to_owned(), u))
    }
}

#[derive(Debug, serde::Deserialize)]
pub struct User {
    pub password: Password,
    pub allowed_calls: HashSet<String>,
    #[serde(default)]
    pub fetch_blocks: bool,
    pub override_wallet: Option<String>,
}
impl User {
    pub async fn intercept<'a>(
        &self,
        state: Arc<State>,
        req: &'a RpcRequest<GenericRpcMethod>,
    ) -> Result<Option<RpcResponse<GenericRpcMethod>>, RpcError> {
        if self.allowed_calls.contains(&*req.method) {
            if self.fetch_blocks && &*req.method == GetBlock.as_str()
            // only non-verbose for now
            {
                match req.params.get(1).unwrap_or(&1_u64.into()) {
                    Value::Number(ref n) if n.as_u64() == Some(0) => {
                        match fetch_block(
                            state.clone(),
                            state.get_peers().await?,
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
                            async {
                                state
                                    .rpc_client
                                    .call(&fetch_header_req)
                                    .await?
                                    .into_result()
                            },
                            async {
                                fetch_block(state.clone(), state.clone().get_peers().await?, hash)
                                    .await
                            }
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
            } else if self.fetch_blocks && &*req.method == GetBlockchainInfo.as_str() {
                let mut res = state.rpc_client.call(req).await?;
                res.result.as_mut().map(|r| match r {
                    Value::Object(o) => o.get_mut("pruned").map(|p| *p = Value::Bool(false)),
                    _ => None,
                });

                Ok(Some(res))
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

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};
    use std::convert::TryInto;

    fn check(input: Option<bool>, default: bool, expected: bool) {
        let mut users = HashMap::new();
        users.insert("satoshi".to_owned(), super::input::User {
            password: "secret".try_into().expect("failed to create password"),
            allowed_calls: HashSet::new(),
            fetch_blocks: input,
        });

        let result = super::input::map_default(users, default);
        assert_eq!(result.0["satoshi"].fetch_blocks, expected);
    }

    #[test]
    fn default_fetch_blocks_none_false() {
        check(None, false, false);
    }

    #[test]
    fn default_fetch_blocks_none_true() {
        check(None, true, true);
    }

    #[test]
    fn default_fetch_blocks_some_false_false() {
        check(Some(false), false, false);
    }

    #[test]
    fn default_fetch_blocks_some_false_true() {
        check(Some(false), true, false);
    }

    #[test]
    fn default_fetch_blocks_some_true_false() {
        check(Some(true), false, true);
    }

    #[test]
    fn default_fetch_blocks_some_true_true() {
        check(Some(true), true, true);
    }
}

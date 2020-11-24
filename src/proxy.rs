use std::sync::Arc;

use anyhow::Error;
use hyper::{
    body::Bytes,
    header::{AUTHORIZATION, WWW_AUTHENTICATE},
    Body, Method, Request, Response, StatusCode,
};
use tokio::stream::StreamExt;

use crate::client::{RpcError, RpcResponse};
use crate::env::Env;

pub async fn proxy_request(env: Arc<Env>, request: Request<Body>) -> Result<Response<Body>, Error> {
    let (parts, body) = request.into_parts();
    if parts.uri.path() == "/" || parts.uri.path() == "" || parts.uri.path().starts_with("/wallet/")
    {
        if parts.method == Method::POST {
            let env_local = env.clone();
            if let Some((name, user)) = parts
                .headers
                .get(AUTHORIZATION)
                .and_then(|auth| env_local.users.get(auth))
            {
                let body_data = body.collect::<Result<Bytes, _>>().await?;
                match serde_json::from_slice(body_data.as_ref()) {
                    Ok(req) => {
                        let env_local = env.clone();
                        let name_local = Arc::new(name);
                        let response = env
                            .rpc_client
                            .send(parts.uri.path(), &req, move |_path, req| {
                                use futures::TryFutureExt;
                                let name_local_ok = name_local.clone();
                                let name_local_err = name_local.clone();
                                let env_local_ok = env_local.clone();
                                let env_local_err = env_local.clone();
                                user.intercept(env_local.clone(), req)
                                    .map_ok(move |res| {
                                        if res.is_some() {
                                            info!(
                                                env_local_ok.logger,
                                                "{} called {}: INTERCEPTED",
                                                name_local_ok,
                                                req.method.0
                                            )
                                        } else {
                                            info!(
                                                env_local_ok.logger,
                                                "{} called {}: FORWARDED",
                                                name_local_ok,
                                                req.method.0
                                            )
                                        }
                                        res
                                    })
                                    .map_err(move |err| {
                                        info!(
                                            env_local_err.logger,
                                            "{} called {}: ERROR {} {}",
                                            name_local_err,
                                            req.method.0,
                                            err.code,
                                            err.message
                                        );
                                        err
                                    })
                            })
                            .await?;
                        Ok(response)
                    }
                    Err(e) => Ok(RpcResponse::from(RpcError::from(e)).into_response()?),
                }
            } else {
                Ok(Response::builder()
                    .status(StatusCode::UNAUTHORIZED)
                    .header(WWW_AUTHENTICATE, "Basic realm=\"jsonrpc\"")
                    .body(Body::empty())?)
            }
        } else {
            Ok(Response::builder()
                .status(StatusCode::METHOD_NOT_ALLOWED)
                .body("JSONRPC server handles only POST requests".into())?)
        }
    } else {
        Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::empty())?)
    }
}

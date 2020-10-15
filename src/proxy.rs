use anyhow::Error;
use hyper::{body::Bytes, header::AUTHORIZATION, Body, Method, Request, Response, StatusCode};
use tokio::stream::StreamExt;

use crate::client::{RpcError, RpcResponse};
use crate::env::Env;

pub async fn proxy_request(
    env: &'static Env,
    request: Request<Body>,
) -> Result<Response<Body>, Error> {
    let (parts, body) = request.into_parts();
    if parts.uri.path() == "/" || parts.uri.path() == "" {
        if parts.method == Method::POST {
            if let Some(user) = parts
                .headers
                .get(AUTHORIZATION)
                .and_then(|auth| env.users.get(auth))
            {
                match serde_json::from_slice(body.collect::<Result<Bytes, _>>().await?.as_ref()) {
                    Ok(req) => Ok(env
                        .rpc_client
                        .send(&req, move |req| user.intercept(env, req))
                        .await?),
                    Err(e) => Ok(RpcResponse::from(RpcError::from(e)).into_response()?),
                }
            } else {
                Ok(Response::builder()
                    .status(StatusCode::UNAUTHORIZED)
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

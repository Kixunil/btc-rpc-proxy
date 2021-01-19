use std::sync::Arc;

use anyhow::Error;
use hyper::{
    body::to_bytes,
    header::{AUTHORIZATION, WWW_AUTHENTICATE},
    Body, Method, Request, Response, StatusCode,
};

use crate::client::{RpcError, RpcResponse};
use crate::state::State;

pub async fn proxy_request(
    state: Arc<State>,
    request: Request<Body>,
) -> Result<Response<Body>, Error> {
    let (parts, body) = request.into_parts();
    if parts.uri.path() == "/" || parts.uri.path() == "" || parts.uri.path().starts_with("/wallet/")
    {
        if parts.method == Method::POST {
            let state_local = state.clone();
            if let Some((name, user)) = parts
                .headers
                .get(AUTHORIZATION)
                .and_then(|auth| state_local.users.get(auth))
            {
                let body_data = to_bytes(body).await?;
                match serde_json::from_slice(body_data.as_ref()) {
                    Ok(req) => {
                        let state_local = state.clone();
                        let name_local = Arc::new(name);
                        let response = state
                            .rpc_client
                            .send(parts.uri.path(), &req, move |_path, req| {
                                use futures::TryFutureExt;
                                let name_local_ok = name_local.clone();
                                let name_local_err = name_local.clone();
                                let state_local_ok = state_local.clone();
                                let state_local_err = state_local.clone();
                                user.intercept(state_local.clone(), req)
                                    .map_ok(move |res| {
                                        let action_description = if res.is_some() {
                                            "INTERCEPTED"
                                        } else {
                                            "FORWARDED"
                                        };

                                        debug!(
                                            state_local_ok.logger,
                                            "processed request";
                                            "user" => name_local_ok,
                                            "method" => req.method.0.clone(),
                                            "action" => action_description
                                        );
                                        res
                                    })
                                    .map_err(move |error| {
                                        warn!(
                                            state_local_err.logger,
                                            "failed request";
                                            "user" => name_local_err,
                                            "method" => req.method.0.clone(),
                                            "error_code" => error.code,
                                            "error_message" => &error.message
                                        );
                                        error
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

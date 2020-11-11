#[macro_use]
extern crate derive_more;
#[macro_use]
extern crate slog;

pub mod client;
pub mod env;
pub mod fetch_blocks;
pub mod proxy;
pub mod rpc_methods;
pub mod users;
pub mod util;

use std::convert::Infallible;
use std::sync::Arc;

use anyhow::Error;
use futures::FutureExt;
use hyper::{
    service::{make_service_fn, service_fn},
    Server,
};

pub use crate::client::{AuthSource, RpcClient};
pub use crate::env::{Env, TorEnv};
pub use crate::fetch_blocks::PeerList;
use crate::proxy::proxy_request;
pub use crate::users::{User, Users};

pub async fn main(env: Arc<Env>) -> Result<(), Error> {
    let env_local = env.clone();
    let make_service = make_service_fn(move |_conn| {
        let env_local_local = env_local.clone();
        async move {
            Ok::<_, Infallible>(service_fn(move |req| {
                proxy_request(env_local_local.clone(), req).boxed()
            }))
        }
    });

    let server = Server::bind(&env.bind).serve(make_service);

    Ok(server.await?)
}

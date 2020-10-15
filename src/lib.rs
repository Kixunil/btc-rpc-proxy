#[macro_use]
extern crate derive_more;
#[macro_use]
extern crate slog;

pub mod client;
pub mod env;
pub mod fetch_blocks;
pub mod proxy;
pub mod users;
pub mod util;

use std::convert::Infallible;

use anyhow::Error;
use futures::FutureExt;
use hyper::{
    service::{make_service_fn, service_fn},
    Server,
};

pub use crate::client::{AuthSource, RpcClient};
pub use crate::env::Env;
pub use crate::fetch_blocks::PeerList;
use crate::proxy::proxy_request;
pub use crate::users::{User, Users};

pub async fn main(env: &'static Env) -> Result<(), Error> {
    let make_service = make_service_fn(move |_conn| async move {
        Ok::<_, Infallible>(service_fn(move |req| proxy_request(env, req).boxed()))
    });

    let server = Server::bind(&env.bind).serve(make_service);

    Ok(server.await?)
}

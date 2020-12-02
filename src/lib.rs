#![cfg_attr(feature = "old_rust", allow(unstable_name_collisions))]

#[macro_use]
extern crate derive_more;
#[macro_use]
extern crate slog;

pub mod client;
pub mod fetch_blocks;
pub mod proxy;
pub mod rpc_methods;
pub mod state;
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
pub use crate::fetch_blocks::Peers;
use crate::proxy::proxy_request;
pub use crate::state::{State, TorState};
pub use crate::users::{User, Users};

pub async fn main(state: Arc<State>) -> Result<(), Error> {
    let state_local = state.clone();
    let make_service = make_service_fn(move |_conn| {
        let state_local_local = state_local.clone();
        async move {
            Ok::<_, Infallible>(service_fn(move |req| {
                proxy_request(state_local_local.clone(), req).boxed()
            }))
        }
    });

    let server = Server::bind(&state.bind).serve(make_service);

    Ok(server.await?)
}

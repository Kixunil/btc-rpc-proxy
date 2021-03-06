#![cfg_attr(feature = "old_rust", allow(unstable_name_collisions))]

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
    server::Server,
    service::{make_service_fn, service_fn},
};
use tokio_compat_02::FutureExt as _;

pub use crate::client::{AuthSource, RpcClient};
pub use crate::fetch_blocks::Peers;
use crate::proxy::proxy_request;
pub use crate::state::{State, TorState};
pub use crate::users::{User, Users};

pub async fn main(state: Arc<State>, bind_addr: systemd_socket::SocketAddr) -> Result<(), Error> {
    let state_local = state.clone();
    let make_service = make_service_fn(move |_conn| {
        let state_local_local = state_local.clone();
        async move {
            Ok::<_, Infallible>(service_fn(move |req| {
                proxy_request(state_local_local.clone(), req).boxed()
            }))
        }
    });

    let listener = bind_addr.bind_tokio_0_2().compat().await.map_err(|error| {
        let new_error = anyhow::anyhow!("failed to create the listening socket: {}", error);
        error!(state.logger, "failed to create the listening socket"; "error" => #error);
        new_error
    })?;

    let server = Server::builder(hyper::server::accept::from_stream(TcpListenerCompat(
        listener,
    )))
    .serve(make_service);

    Ok(server.await?)
}

struct TcpListenerCompat(tokio_02::net::TcpListener);
impl futures::Stream for TcpListenerCompat {
    type Item = tokio_02::io::Result<tokio_compat_02::IoCompat<tokio_02::net::TcpStream>>;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        tokio_02::stream::Stream::poll_next(unsafe { self.map_unchecked_mut(|s| &mut s.0) }, cx)
            .map(|a| a.map(|a| a.map(tokio_compat_02::IoCompat::new)))
    }
}

#[macro_use]
extern crate configure_me;
#[macro_use]
extern crate serde;

use anyhow::Error;
use btc_rpc_proxy;

mod create_env;

#[tokio::main]
async fn main() -> Result<(), Error> {
    let env = create_env::create_env().await?.arc();
    btc_rpc_proxy::main(env).await
}

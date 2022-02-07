// mod cli;
// mod client;
mod crypto;
mod display;
mod proto;
mod sdk;
mod utils;
// mod wallet;
mod cmd;
mod config;
mod interactive;

use cmd::all_cmd;
use config::Config;
use crypto::EthCrypto;
use crypto::SmCrypto;

use anyhow::Result;

fn main() -> Result<()> {
    let config = Config {
        controller_addr: "localhost:50005".into(),
        executor_addr: "localhost:50002".into(),
        default_account: None,
        wallet_dir: "d:/cld/cloud-cli/tmp-wallet".into(),
    };

    let mut ctx = sdk::context::from_config::<SmCrypto>(&config).unwrap();
    let mut cmd = all_cmd();

    cmd.exec(&mut ctx).map_err(|e| {
        if let Some(e) = e.downcast_ref::<clap::Error>() {
            e.exit();
        }
        e
    })
}

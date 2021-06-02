#[cfg(all(feature = "evm", feature = "chaincode"))]
compile_error!("features `evm` and `chaincode` are mutually exclusive");

#[cfg(all(feature = "sm", feature = "eth"))]
compile_error!("features `sm` and `eth` are mutually exclusive");

mod cli;
mod client;
mod crypto;
mod display;
mod interactive;
mod util;
mod wallet;

use rand::{thread_rng, Rng};
use std::sync::Arc;
use std::time::Duration;

use cli::build_cli;
use client::Client;
use display::Display as _;
use futures::future::join_all;
use interactive::Interactive;
use util::{parse_addr, parse_data, parse_value};
use wallet::Wallet;

use anyhow::anyhow;
use anyhow::Result;

/// Store action target address
pub const STORE_ADDRESS: &str = "0xffffffffffffffffffffffffffffffffff010000";
/// StoreAbi action target address
pub const ABI_ADDRESS: &str = "0xffffffffffffffffffffffffffffffffff010001";
/// Amend action target address
pub const AMEND_ADDRESS: &str = "0xffffffffffffffffffffffffffffffffff010002";

/// amend the abi data
pub const AMEND_ABI: &str = "0x01";
/// amend the account code
pub const AMEND_CODE: &str = "0x02";
/// amend the kv of db
pub const AMEND_KV_H256: &str = "0x03";
/// amend account balance
pub const AMEND_BALANCE: &str = "0x05";

#[tokio::main]
async fn main() -> Result<()> {
    // security not included yet:p
    let cli = build_cli();

    let matches = cli.get_matches();

    let user = matches
        .value_of("user")
        .map(str::to_string)
        .unwrap_or_else(|| {
            if let Ok(user) = std::env::var("CITA_CLOUD_USER") {
                user
            } else {
                "default".to_string()
            }
        });

    let controller_addr = matches
        .value_of("controller_addr")
        .map(str::to_string)
        .unwrap_or_else(|| {
            if let Ok(controller_addr) = std::env::var("CITA_CLOUD_CONTROLLER_ADDR") {
                controller_addr
            } else {
                "localhost:50004".to_string()
            }
        });
    let executor_addr = matches
        .value_of("executor_addr")
        .map(str::to_string)
        .unwrap_or_else(|| {
            if let Ok(executor_addr) = std::env::var("CITA_CLOUD_EXECUTOR_ADDR") {
                executor_addr
            } else {
                "localhost:50002".to_string()
            }
        });

    let wallet = {
        let data_dir = {
            let home = home::home_dir().expect("cannot find home dir");
            home.join(".cloud-cli")
        };
        Wallet::open(data_dir)
    };

    let account = match wallet.load_account(&user) {
        Some(account) => account,
        None => return Err(anyhow!("account no found")),
    };

    let client = Arc::new(Client::new(account, &controller_addr, &executor_addr));

    if let Some(subcmd) = matches.subcommand() {
        match subcmd {
            ("call", m) => {
                let from = parse_addr(m.value_of("from").unwrap_or_default())?;
                let to = parse_addr(m.value_of("to").unwrap())?;
                let data = parse_data(m.value_of("data").unwrap())?;

                let result = client.call(from, to, data).await;
                println!("result: 0x{}", hex::encode(&result));
            }
            ("send", m) => {
                let to = parse_addr(m.value_of("to").unwrap())?;
                let data = parse_data(m.value_of("data").unwrap())?;
                let value = parse_value(m.value_of("value").unwrap_or_default())?;

                let tx_hash = client.send(to, data, value).await;
                println!("tx_hash: 0x{}", hex::encode(&tx_hash));
            }
            ("block_number", m) => {
                let for_pending = m.is_present("for_pending");

                let block_number = client.get_block_number(for_pending).await;
                println!("block_number: {}", block_number);
            }
            ("block_at", m) => {
                let block_number = m.value_of("block_number").unwrap().parse::<u64>()?;

                let block = client.get_block_by_number(block_number).await;
                println!("{}", block.display());
            }
            ("get_tx", m) => {
                let tx_hash = parse_value(m.value_of("tx_hash").unwrap())?;

                let tx = client.get_tx(tx_hash).await;
                println!("tx: {}", tx.display());
            }
            ("peer_count", _m) => {
                let cnt = client.get_peer_count().await;
                println!("peer_count: {}", cnt);
            }
            ("system_config", _m) => {
                let system_config = client.get_system_config().await;
                println!("{}", system_config.display());
            }
            ("bench", m) => {
                let tx_count = m.value_of("count").unwrap().parse::<u64>()?;

                let mut start_at = client.get_block_number(false).await;

                let mut rng = thread_rng();
                let handles = (0..tx_count)
                    .map(|_| {
                        let client = Arc::clone(&client);

                        let to: [u8; 20] = rng.gen();
                        let data: [u8; 32] = rng.gen();
                        let value: [u8; 32] = rng.gen();
                        tokio::spawn(async move {
                            client.send(to.into(), data.into(), value.into()).await
                        })
                    })
                    .collect::<Vec<_>>();
                join_all(handles).await;

                println!("sending txs done.");

                let mut check_interval = tokio::time::interval(Duration::from_secs(1));
                let mut finalized_tx = 0;
                let mut begin_time = None;

                while finalized_tx < tx_count {
                    check_interval.tick().await;
                    let end_at = {
                        let n = client.get_block_number(false).await;
                        if n >= start_at {
                            n
                        } else {
                            continue;
                        }
                    };

                    let blocks = {
                        let handles = (start_at..=end_at)
                            .map(|n| {
                                let client = Arc::clone(&client);
                                tokio::spawn(async move { client.get_block_by_number(n).await })
                            })
                            .collect::<Vec<_>>();
                        join_all(handles).await
                    };

                    for b in blocks {
                        let b = b.unwrap();
                        let (header, body) = (b.header.unwrap(), b.body.unwrap());

                        let height = header.height;
                        let secs = {
                            let t = std::time::UNIX_EPOCH + Duration::from_millis(header.timestamp);
                            if begin_time.is_none() {
                                begin_time.replace(t);
                            }
                            t.duration_since(begin_time.unwrap()).unwrap().as_secs()
                        };
                        let cnt = body.tx_hashes.len() as u64;
                        finalized_tx += cnt;
                        println!(
                            "{:0>2}:{:0>2} block `{}` contains `{}` txs, finalized: `{}`",
                            secs / 60,
                            secs % 60,
                            height,
                            cnt,
                            finalized_tx
                        );
                    }
                    start_at = end_at + 1;
                }
            }
            ("account", m) => {
                if let Some(subcmd) = m.subcommand() {
                    match subcmd {
                        ("create", m) => {
                            let user = m.value_of("user").unwrap();
                            let addr = wallet.create_account(user);
                            println!("user: `{}`\naccount_addr: 0x{}", user, hex::encode(&addr));
                        }
                        ("login", m) => {
                            let user = m.value_of("user").unwrap();
                            let addr = wallet.set_default_user(user)?;
                            println!(
                                "OK, now the default user is `{}`, account addr is 0x{}",
                                user,
                                hex::encode(&addr)
                            );
                        }
                        ("import", m) => {
                            let user = m.value_of("user").unwrap();
                            let pk = parse_data(m.value_of("pk").unwrap())?;
                            let sk = parse_data(m.value_of("sk").unwrap())?;
                            wallet.import_account(user, pk, sk);
                            println!("OK, account imported");
                        }
                        ("export", m) => {
                            let user = m.value_of("user").unwrap();
                            if let Some(account) = wallet.load_account(user) {
                                println!("{}", account.display());
                            } else {
                                println!("No such an account");
                            }
                        }
                        ("delete", m) => {
                            let user = m.value_of("user").unwrap();
                            wallet.delete_account(user);
                            println!("Ok, the account of user `{}` has been deleted", user);
                        }
                        _ => unreachable!(),
                    }
                } else {
                    println!("users: {:#?}", wallet.list_account());
                }
            }
            ("completions", m) => {
                use clap_generate::{generate, generators::*};
                use std::io;
                let shell = m.value_of("shell").unwrap();
                let mut cli = cli::build_cli();
                let mut stdout = io::stdout();
                match shell {
                    "bash" => generate::<Bash, _>(&mut cli, "cldi", &mut stdout),
                    "powershell" => generate::<PowerShell, _>(&mut cli, "cldi", &mut stdout),
                    "zsh" => generate::<Zsh, _>(&mut cli, "cldi", &mut stdout),
                    "fish" => generate::<Fish, _>(&mut cli, "cldi", &mut stdout),
                    "elvish" => generate::<Elvish, _>(&mut cli, "cldi", &mut stdout),
                    _ => unreachable!(),
                }
            }
            #[cfg(feature = "evm")]
            ("create", m) => {
                let to = vec![];
                let data = parse_data(m.value_of("data").unwrap())?;
                let value = parse_value(m.value_of("value").unwrap_or_default())?;

                let tx_hash = client.send(to, data, value).await;
                println!("tx_hash: 0x{}", hex::encode(&tx_hash));
            }
            #[cfg(feature = "evm")]
            ("receipt", m) => {
                let tx_hash = parse_value(m.value_of("tx_hash").unwrap())?;

                let receipt = client.get_receipt(tx_hash).await;
                println!("{}", receipt.display());
            }
            #[cfg(feature = "evm")]
            ("get_code", m) => {
                let addr = parse_addr(m.value_of("addr").unwrap())?;

                let code = client.get_code(addr).await;
                println!("code: 0x{}", hex::encode(&code.byte_code));
            }
            #[cfg(feature = "evm")]
            ("get_balance", m) => {
                let addr = parse_addr(m.value_of("addr").unwrap())?;

                let balance = client.get_balance(addr).await;
                println!("balance: 0x{}", hex::encode(&balance.value));
            }
            #[cfg(feature = "evm")]
            ("store_abi", m) => {
                let to = parse_addr(ABI_ADDRESS)?;
                let data = {
                    let addr = parse_addr(m.value_of("addr").unwrap())?;
                    let abi = m.value_of("abi").unwrap();

                    // [<addr><abi>]
                    [addr.as_slice(), abi.as_bytes()].concat()
                };

                let tx_hash = client.send(to, data, vec![0; 32]).await;
                println!("tx_hash: 0x{}", hex::encode(&tx_hash));
            }
            #[cfg(feature = "evm")]
            ("get_abi", m) => {
                let addr = parse_addr(m.value_of("addr").unwrap())?;
                let abi = client.get_abi(addr).await;

                println!("ABI: {}", String::from_utf8(abi.bytes_abi)?);
            }
            _ => {
                unreachable!()
            }
        }
    } else {
        Interactive::run()
    }

    Ok(())
}

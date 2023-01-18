use anyhow::{anyhow, Context};
use cardano_multiplatform_lib::address::StakeCredential;

use cardano_multiplatform_lib::crypto::{Ed25519KeyHash, ScriptHash};
use clap::Parser;
use pallas_addresses::{ShelleyDelegationPart, ShelleyPaymentPart};
use serde::Deserialize;
use std::collections::HashSet;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use utxo_selection_benchmark::generation_utils::clean_events;
use utxo_selection_benchmark::mapper::DataMapper;

use utxo_selection_benchmark::utils::{dump_hashset_to_file, read_hashset_from_file};

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    events_path: PathBuf,
    cleaned_events_output_path: PathBuf,

    unparsed_transaction_addresses: PathBuf,

    payment_creds_mapping: PathBuf,
    payment_creds_mapping_output: PathBuf,
    staking_creds_mapping: PathBuf,
    staking_creds_mapping_output: PathBuf,
    banned_addresses: PathBuf,
    banned_addresses_output: PathBuf,
}

#[derive(Parser, Debug)]
#[clap(version)]
pub struct Cli {
    /// path to config file
    #[clap(long, value_parser)]
    config_path: PathBuf,
}

#[tokio::main]
async fn main() {
    let result = _main().await;
    result.unwrap();
}

async fn _main() -> anyhow::Result<()> {
    // Start logging setup block
    let fmt_layer = tracing_subscriber::fmt::layer().with_test_writer();

    tracing_subscriber::registry().with(fmt_layer).init();

    let Cli { config_path } = Cli::parse();

    tracing::info!("Config file {:?}", config_path);
    let file = File::open(&config_path).with_context(|| {
        format!(
            "Cannot read config file {path}",
            path = config_path.display()
        )
    })?;
    let config: Config = serde_yaml::from_reader(file).with_context(|| {
        format!(
            "Cannot read config file {path}",
            path = config_path.display()
        )
    })?;

    let unparsed_addresses_file = if config.unparsed_transaction_addresses.exists()
        && config.unparsed_transaction_addresses.is_file()
    {
        File::open(config.unparsed_transaction_addresses.clone())?
    } else {
        return Err(anyhow!(
            "can't open input file: {:?}",
            config.unparsed_transaction_addresses
        ));
    };

    tracing::info!("loading mappings");

    let mut stake_address_to_num =
        DataMapper::<StakeCredential>::load_from_file(config.staking_creds_mapping)?;
    tracing::info!("stake addresses loaded");

    let mut payment_address_to_num =
        DataMapper::<StakeCredential>::load_from_file(config.payment_creds_mapping)?;
    tracing::info!("payment addresses loaded");

    let mut banned_addresses: HashSet<(u64, Option<u64>)> =
        read_hashset_from_file(config.banned_addresses)?;
    tracing::info!("banned addresses loaded");

    tracing::info!("successfully loaded mappings");

    let unparsed_addresses_file_lines = BufReader::new(unparsed_addresses_file).lines();
    for line in unparsed_addresses_file_lines {
        let address = line?;

        match pallas_addresses::Address::from_bech32(address.as_str()) {
            Ok(address) => {
                let (payment, staking) = match address {
                    pallas_addresses::Address::Byron(_) => {
                        /* ignore */
                        continue;
                    }
                    pallas_addresses::Address::Shelley(shelley) => {
                        let payment_cred = match shelley.payment() {
                            ShelleyPaymentPart::Key(key) => StakeCredential::from_keyhash(
                                &Ed25519KeyHash::from_bytes(key.to_vec()).unwrap(),
                            ),
                            ShelleyPaymentPart::Script(script) => StakeCredential::from_scripthash(
                                &ScriptHash::from_bytes(script.to_vec()).unwrap(),
                            ),
                        };
                        let staking_cred: Option<StakeCredential> = match shelley.delegation() {
                            ShelleyDelegationPart::Null => None,
                            ShelleyDelegationPart::Key(key) => Some(StakeCredential::from_keyhash(
                                &Ed25519KeyHash::from_bytes(key.to_vec()).unwrap(),
                            )),
                            ShelleyDelegationPart::Script(script) => {
                                Some(StakeCredential::from_scripthash(
                                    &ScriptHash::from_bytes(script.to_vec()).unwrap(),
                                ))
                            }
                            ShelleyDelegationPart::Pointer(_) => {
                                todo!("not supported")
                            }
                        };
                        (payment_cred, staking_cred)
                    }
                    pallas_addresses::Address::Stake(_stake) => {
                        /* ignore */
                        continue;
                    }
                };
                let payment_mapping = payment_address_to_num.add_if_not_presented(payment);
                let staking_mapping =
                    staking.map(|staking| stake_address_to_num.add_if_not_presented(staking));
                banned_addresses.insert((payment_mapping, staking_mapping));
            }

            Err(err) => {
                tracing::error!("can't parse address: {:?}, addr={:?}", err, address);
            }
        }
    }

    tracing::info!("Parsing finished, dumping files");

    payment_address_to_num.dump_to_file(config.payment_creds_mapping_output)?;
    stake_address_to_num.dump_to_file(config.staking_creds_mapping_output)?;
    dump_hashset_to_file(&banned_addresses, config.banned_addresses_output)?;

    tracing::info!("Dumping finished, cleaning events");

    clean_events(
        config.events_path,
        config.cleaned_events_output_path,
        &banned_addresses,
    )?;

    tracing::info!("Cleaning finished");

    Ok(())
}

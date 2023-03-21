use std::path::PathBuf;

use anyhow::Context;

use clap::Parser;
use serde::Deserialize;

use dcspark_core::network_id::NetworkInfo;
use std::fs::File;

use tracing_subscriber::prelude::*;
use utxo_selection::algorithms::ThermostatAlgoConfig;
use utxo_selection::estimators::ThermostatFeeEstimator;
use utxo_selection_benchmark::bench::run_algorithm_benchmark;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    events_path: PathBuf,
    output_insolvent: PathBuf,
    output_discarded: PathBuf,
    output_balance: PathBuf,
    output_balance_short: PathBuf,
    #[serde(default)]
    utxos_path: Option<PathBuf>,
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

    let io_selection_algo =
        utxo_selection::algorithms::Thermostat::new(ThermostatAlgoConfig::default());

    let change_balance_algo =
        utxo_selection::algorithms::Thermostat::new(ThermostatAlgoConfig::default());

    run_algorithm_benchmark(
        io_selection_algo,
        change_balance_algo,
        || {
            Ok(ThermostatFeeEstimator::new(
                NetworkInfo::Mainnet,
                &serde_json::from_str(
                    "
                {
                    \"quorum\": 3,
                    \"keys\": [
                        \"ecbb34d9e8f0356107036153babcb1e01b43d4ed5b849b15dfd8f6a5\",
                        \"c7cb0f556a68766e672835108b5f43be9a193c9063866db6c50b5e25\",
                        \"937d1f47dba39cb547287fd786db966ecd70489c6c35494018c69144\",
                        \"721663acd63a5b3644e8c4e4ce7649457fd671922f448b497a4fd25d\",
                        \"11a39984271cc3f0714f241e2e6df15e8abf2974f3772ffbeefb7a36\"
                    ]
                }",
                )?,
            ))
        },
        config.events_path,
        config.output_insolvent,
        config.output_discarded,
        config.output_balance,
        config.output_balance_short,
        false,
        config.utxos_path,
    )?;
    Ok(())
}

use std::path::PathBuf;

use anyhow::Context;
use cardano_utils::multisig_plan::MultisigPlan;
use clap::Parser;
use serde::Deserialize;

use std::fs::File;

use tracing_subscriber::prelude::*;
use utxo_selection::algorithms::{ThermostatAlgoConfig, ThermostatFeeEstimator};
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
                cardano_utils::network_id::NetworkInfo::Mainnet,
                &MultisigPlan {
                    quorum: 0,
                    keys: vec![],
                },
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

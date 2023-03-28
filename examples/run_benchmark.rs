use std::path::PathBuf;

use anyhow::Context;

use clap::Parser;

use dcspark_core::network_id::NetworkInfo;
use dcspark_core::UTxOStore;
use std::fs::File;

use tracing_subscriber::prelude::*;
use utxo_selection::algorithms::ThermostatAlgoConfig;

use utxo_selection::estimators::ThermostatFeeEstimator;
use utxo_selection_benchmark::bench::{run_algorithm_benchmark, PathsConfig};
use utxo_selection_benchmark::bench_utils::address_mapper::StringAddressMapper;
use utxo_selection_benchmark::bench_utils::selection_eligibility::SelectionEligibility;

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
    let config: PathsConfig = serde_yaml::from_reader(file).with_context(|| {
        format!(
            "Cannot read config file {path}",
            path = config_path.display()
        )
    })?;

    let thermostat = utxo_selection::algorithms::Thermostat::new(ThermostatAlgoConfig::default());
    let thermostat_fee_estimator = || {
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
    };

    let _largest_first = utxo_selection::algorithms::LargestFirst::try_from(UTxOStore::new())?;

    let single_change = utxo_selection::algorithms::SingleOutputChangeBalancer::default();

    let mut selection = SelectionEligibility::default();
    selection.set_staking_keys_of_interest(vec![9999999]);

    run_algorithm_benchmark(
        thermostat,
        single_change,
        thermostat_fee_estimator,
        StringAddressMapper::default(),
        selection,
        config,
        false,
    )?;
    Ok(())
}

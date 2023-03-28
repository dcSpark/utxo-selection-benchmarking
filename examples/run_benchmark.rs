use std::path::PathBuf;

use anyhow::{anyhow, Context};

use clap::Parser;

use cardano_multiplatform_lib::builders::tx_builder::TransactionBuilderConfigBuilder;
use cardano_multiplatform_lib::builders::witness_builder::NativeScriptWitnessInfo;
use cardano_multiplatform_lib::ledger::alonzo::fees::LinearFee;
use cardano_multiplatform_lib::ledger::common::value::BigNum;
use cardano_multiplatform_lib::metadata::{
    AuxiliaryData, GeneralTransactionMetadata, TransactionMetadatum,
};
use cardano_multiplatform_lib::plutus::{ExUnitPrices, PlutusData};
use cardano_multiplatform_lib::{RequiredSigners, UnitInterval};
use dcspark_core::multisig_plan::MultisigPlan;
use dcspark_core::network_id::NetworkInfo;
use dcspark_core::tx::{CardanoPaymentCredentials, UTxOBuilder, UTxODetails};
use dcspark_core::{Address, UTxOStore};
use std::fs::File;

use tracing_subscriber::prelude::*;
use utxo_selection::algorithms::ThermostatAlgoConfig;

use utxo_selection::estimators::{CmlFeeEstimator, ThermostatFeeEstimator};
use utxo_selection::{InputSelectionAlgorithm, TransactionFeeEstimator, UTxOStoreSupport};
use utxo_selection_benchmark::bench::{run_algorithm_benchmark, PathsConfig};
use utxo_selection_benchmark::bench_utils::address_mapper::{
    CardanoAddressMapper, CardanoDataMapper, StringAddressMapper,
};
use utxo_selection_benchmark::bench_utils::selection_eligibility::SelectionEligibility;

use serde::Deserialize;

#[derive(Parser, Debug)]
#[clap(version)]
pub struct Cli {
    /// path to config file
    #[clap(long, value_parser)]
    config_path: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
pub enum AlgoConfig {
    LargestFirst,
    Thermostat {
        #[serde(default)]
        config: ThermostatAlgoConfig,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
pub enum BalanceChangeAlgoConfig {
    Fee,
    SingleChange,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
pub enum CardanoCmlEstimatorConfig {
    PlutusScript {
        // TODO: enable plutus script support
        // partial_witness: PartialPlutusWitness,
        required_signers: RequiredSigners,
        datum: PlutusData,
    },
    PaymentKey,
    NativeScript {
        plan: PathBuf,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
pub enum FeeEstimatorConfig {
    Thermostat {
        network: NetworkInfo,
        plan_path: PathBuf,
    },
    CmlEstimator {
        config: CardanoCmlEstimatorConfig,
        magic: Option<String>,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
pub enum DataMapperConfig {
    StringMapper,
    CmlMapper {
        payment_key_path: PathBuf,
        staking_key_path: PathBuf,
        policy_id_path: PathBuf,
        asset_name_path: PathBuf,
        network: u8,
        default_address: Address,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    paths: PathsConfig,

    algo: AlgoConfig,
    change_balance_algo: BalanceChangeAlgoConfig,
    fee_estimator: FeeEstimatorConfig,
    mapper: DataMapperConfig,

    keys_of_interest: Vec<u64>,
    allow_balance_change: bool,
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

    parse_algo(config)
}

pub fn parse_algo(main_config: Config) -> anyhow::Result<()> {
    match main_config.algo.clone() {
        AlgoConfig::LargestFirst => parse_change_algo(
            main_config,
            utxo_selection::algorithms::LargestFirst::try_from(UTxOStore::new())?,
        ),
        AlgoConfig::Thermostat { config } => parse_change_algo(
            main_config,
            utxo_selection::algorithms::Thermostat::new(config),
        ),
    }
}

pub fn parse_change_algo<
    Algo: InputSelectionAlgorithm<InputUtxo = UTxODetails, OutputUtxo = UTxOBuilder> + UTxOStoreSupport,
>(
    main_config: Config,
    algo: Algo,
) -> anyhow::Result<()> {
    match &main_config.change_balance_algo {
        BalanceChangeAlgoConfig::Fee => parse_estimator_creator(
            main_config,
            algo,
            utxo_selection::algorithms::FeeChangeBalancer::default(),
        ),
        BalanceChangeAlgoConfig::SingleChange => parse_estimator_creator(
            main_config,
            algo,
            utxo_selection::algorithms::SingleOutputChangeBalancer::default(),
        ),
    }
}

pub fn parse_estimator_creator<
    Algo: InputSelectionAlgorithm<InputUtxo = UTxODetails, OutputUtxo = UTxOBuilder> + UTxOStoreSupport,
    ChangeAlgo: InputSelectionAlgorithm<InputUtxo = UTxODetails, OutputUtxo = UTxOBuilder> + UTxOStoreSupport,
>(
    main_config: Config,
    algo: Algo,
    change_algo: ChangeAlgo,
) -> anyhow::Result<()> {
    match main_config.fee_estimator.clone() {
        FeeEstimatorConfig::Thermostat { network, plan_path } => {
            let plan = MultisigPlan::load(plan_path)?;
            parse_mapper(main_config, algo, change_algo, || {
                Ok(ThermostatFeeEstimator::new(network.clone(), &plan))
            })
        }
        FeeEstimatorConfig::CmlEstimator { config, magic } => {
            let credentials = match config {
                CardanoCmlEstimatorConfig::PlutusScript { .. } => {
                    todo!("not implemented")
                }
                CardanoCmlEstimatorConfig::PaymentKey => CardanoPaymentCredentials::PaymentKey,
                CardanoCmlEstimatorConfig::NativeScript { plan } => {
                    let plan = MultisigPlan::load(plan)?;
                    CardanoPaymentCredentials::NativeScript {
                        native_script: plan.to_script().get(0),
                        witness_info: NativeScriptWitnessInfo::num_signatures(plan.quorum as usize),
                    }
                }
            };

            parse_mapper(main_config, algo, change_algo, || {
                // TODO: make this configurable
                let coefficient = BigNum::from_str("44").unwrap();
                let constant = BigNum::from_str("155381").unwrap();
                let linear_fee = LinearFee::new(&coefficient, &constant);
                let pool_deposit = BigNum::from_str("500000000").unwrap();
                let key_deposit = BigNum::from_str("2000000").unwrap();
                let max_value_size = 5000;
                let max_tx_size = 16384;
                let coins_per_utxo_byte = BigNum::from_str("4310").unwrap();

                let mut builder =
                    cardano_multiplatform_lib::builders::tx_builder::TransactionBuilder::new(
                        &TransactionBuilderConfigBuilder::new()
                            .fee_algo(&linear_fee)
                            .pool_deposit(&pool_deposit)
                            .key_deposit(&key_deposit)
                            .max_value_size(max_value_size)
                            .max_tx_size(max_tx_size)
                            .coins_per_utxo_byte(&coins_per_utxo_byte)
                            .ex_unit_prices(&ExUnitPrices::new(
                                &UnitInterval::new(&BigNum::zero(), &BigNum::zero()),
                                &UnitInterval::new(&BigNum::zero(), &BigNum::zero()),
                            ))
                            .collateral_percentage(0)
                            .max_collateral_inputs(0)
                            .build()
                            .map_err(|err| anyhow!("can't build tx builder: {}", err))?,
                    );

                if let Some(magic) = &magic {
                    // for the unwrap method we still set the metadata 87 to mark who is the
                    // source of the
                    let auxiliary_data = {
                        let mut auxiliary_data = AuxiliaryData::new();
                        let mut metadata = GeneralTransactionMetadata::new();
                        metadata.insert(
                            &BigNum::from_str("87").expect("87 should read as a bignum"),
                            &TransactionMetadatum::new_text(magic.clone()).map_err(|error| {
                                anyhow::anyhow!("Failed to encode the magic metadata: {}", error)
                            })?,
                        );
                        auxiliary_data.set_metadata(&metadata);
                        auxiliary_data
                    };
                    builder.set_auxiliary_data(&auxiliary_data);
                }

                CmlFeeEstimator::new(builder, credentials.clone(), true)
            })
        }
    }
}

pub fn parse_mapper<
    Estimator: TransactionFeeEstimator<InputUtxo = UTxODetails, OutputUtxo = UTxOBuilder>,
    Algo: InputSelectionAlgorithm<InputUtxo = UTxODetails, OutputUtxo = UTxOBuilder> + UTxOStoreSupport,
    ChangeAlgo: InputSelectionAlgorithm<InputUtxo = UTxODetails, OutputUtxo = UTxOBuilder> + UTxOStoreSupport,
    EstimatorCreator,
>(
    main_config: Config,
    algo: Algo,
    change_algo: ChangeAlgo,
    estimator_creator: EstimatorCreator,
) -> anyhow::Result<()>
where
    EstimatorCreator: Fn() -> anyhow::Result<Estimator>,
{
    match main_config.mapper.clone() {
        DataMapperConfig::StringMapper => {
            run_bench::<Estimator, Algo, ChangeAlgo, EstimatorCreator, StringAddressMapper>(
                main_config,
                algo,
                change_algo,
                estimator_creator,
                StringAddressMapper::default(),
            )
        }
        DataMapperConfig::CmlMapper {
            payment_key_path,
            staking_key_path,
            policy_id_path,
            asset_name_path,
            network,
            default_address,
        } => run_bench::<Estimator, Algo, ChangeAlgo, EstimatorCreator, CardanoAddressMapper>(
            main_config,
            algo,
            change_algo,
            estimator_creator,
            CardanoAddressMapper::new(
                payment_key_path,
                staking_key_path,
                policy_id_path,
                asset_name_path,
                network,
                default_address,
            )?,
        ),
    }
}

pub fn run_bench<
    Estimator: TransactionFeeEstimator<InputUtxo = UTxODetails, OutputUtxo = UTxOBuilder>,
    Algo: InputSelectionAlgorithm<InputUtxo = UTxODetails, OutputUtxo = UTxOBuilder> + UTxOStoreSupport,
    ChangeAlgo: InputSelectionAlgorithm<InputUtxo = UTxODetails, OutputUtxo = UTxOBuilder> + UTxOStoreSupport,
    EstimatorCreator,
    DataMapper: CardanoDataMapper,
>(
    main_config: Config,
    algo: Algo,
    change_algo: ChangeAlgo,
    estimator_creator: EstimatorCreator,
    data_mapper: DataMapper,
) -> anyhow::Result<()>
where
    EstimatorCreator: Fn() -> anyhow::Result<Estimator>,
{
    let mut selection = SelectionEligibility::default();
    if !main_config.keys_of_interest.is_empty() {
        selection.set_staking_keys_of_interest(main_config.keys_of_interest);
    }

    run_algorithm_benchmark(
        algo,
        change_algo,
        estimator_creator,
        data_mapper,
        selection,
        main_config.paths,
        main_config.allow_balance_change,
    )
}

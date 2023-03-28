use std::cell::RefCell;
use std::path::PathBuf;

use crate::tx_event::{TxEvent, TxOutput};

use dcspark_core::tx::{UTxOBuilder, UTxODetails};
use dcspark_core::{Balance, Regulated, TokenId};

use crate::bench_utils::address_mapper::CardanoDataMapper;
use crate::bench_utils::balance_accumulator::BalanceAccumulator;
use crate::bench_utils::balance_verification::verify_io_balance;
use crate::bench_utils::change_extraction::extract_changes;
use crate::bench_utils::output_utils::{builders_to_utxo_details, tx_outputs_to_utxo_builders};
use crate::bench_utils::selection_eligibility::SelectionEligibility;
use crate::bench_utils::utxo_accumulator::UTxOStoreAccumulator;
use serde::Deserialize;

use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::rc::Rc;
use std::str::FromStr;

use crate::bench_utils::stats_accumulator::{BalanceStats, StatsAccumulator};
use crate::utils::balance_to_i64;
use utxo_selection::{
    InputOutputSetup, InputSelectionAlgorithm, TransactionFeeEstimator, UTxOStoreSupport,
};

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathsConfig {
    events_path: PathBuf,

    output_insolvent: PathBuf,
    output_discarded: PathBuf,

    output_balance: PathBuf,
    output_balance_short: PathBuf,

    #[serde(default)]
    utxos_path: Option<PathBuf>,

    #[serde(default)]
    utxos_balance_path: Option<PathBuf>,

    #[serde(default)]
    balance_points_path: Option<PathBuf>,
}

#[allow(clippy::too_many_arguments)]
pub fn run_algorithm_benchmark<
    Estimator: TransactionFeeEstimator<InputUtxo = UTxODetails, OutputUtxo = UTxOBuilder>,
    Algo: InputSelectionAlgorithm<InputUtxo = UTxODetails, OutputUtxo = UTxOBuilder> + UTxOStoreSupport,
    ChangeBalanceAlgo: InputSelectionAlgorithm<InputUtxo = UTxODetails, OutputUtxo = UTxOBuilder> + UTxOStoreSupport,
    EstimatorCreator,
    DataMapper: CardanoDataMapper,
>(
    mut algorithm: Algo,
    mut balance_change_algo: ChangeBalanceAlgo,
    estimator_creator: EstimatorCreator,
    mut data_mapper: DataMapper,
    selection_eligibility_criteria: SelectionEligibility,
    paths: PathsConfig,
    allow_balance_change: bool,
) -> anyhow::Result<()>
where
    EstimatorCreator: Fn() -> anyhow::Result<Estimator>,
{
    let selection_eligibility_criteria = Rc::new(RefCell::new(selection_eligibility_criteria));

    let mut actual_balance_acc = BalanceAccumulator::new(selection_eligibility_criteria.clone());
    let mut computed_balance_acc = BalanceAccumulator::new(selection_eligibility_criteria.clone());

    let mut utxo_accumulator = UTxOStoreAccumulator::new(selection_eligibility_criteria.clone());

    let input_events = BufReader::new(File::open(paths.events_path.clone())?);

    let mut balance_points_acc = StatsAccumulator::<BalanceStats>::default();
    let mut utxo_count_acc = StatsAccumulator::<u64>::default();
    let mut read: u64 = 0;

    for (tx_number, event_str) in input_events.lines().enumerate() {
        read += 1;

        for stake_key in selection_eligibility_criteria
            .clone()
            .as_ref()
            .borrow()
            .get_whitelisted_non_banned()
            .iter()
        {
            collect_stats(
                stake_key,
                tx_number as u64,
                &paths,
                &actual_balance_acc,
                &computed_balance_acc,
                &utxo_accumulator,
                &mut balance_points_acc,
                &mut utxo_count_acc,
            );
        }

        let event: TxEvent = serde_json::from_str(&event_str?)?;
        match event {
            TxEvent::Full {
                from: inputs,
                fee,
                to: outputs,
            } => {
                verify_io_balance(&inputs, &outputs, &fee).unwrap(); // if balance is not correct -> data is corrupted

                actual_balance_acc.reduce_balance_from(&inputs, &mut data_mapper)?;
                actual_balance_acc.add_balance_from(&outputs, &mut data_mapper)?;

                let should_perform_selection = selection_eligibility_criteria
                    .clone()
                    .borrow_mut()
                    .should_perform_selection(&inputs);

                let (pk, sk) = match should_perform_selection {
                    None => {
                        remove_inputs_from_consideration(
                            inputs,
                            &mut utxo_accumulator,
                            &mut actual_balance_acc,
                            &mut computed_balance_acc,
                            selection_eligibility_criteria.clone(),
                        );
                        add_balances_from_partial_outputs(
                            tx_number as u64,
                            outputs,
                            &mut utxo_accumulator,
                            &mut computed_balance_acc,
                            &mut data_mapper,
                        )?;
                        continue;
                    }
                    Some(keys) => keys,
                };

                actual_balance_acc.add_fee_spending(sk, &fee);

                let pk = pk.first().cloned().unwrap(); // pk must exist, since we've found sk

                // now we have inputs related to only one staking key. we're not insolvent and not discarded

                let parsed_outputs = extract_changes(&outputs, (pk, sk));
                let non_change_outputs =
                    tx_outputs_to_utxo_builders(parsed_outputs.fixed_outputs, &mut data_mapper)?;

                let mut estimate = estimator_creator()?;

                for output in non_change_outputs.iter() {
                    estimate.add_output(output.clone())?;
                }

                let available_inputs = utxo_accumulator.get_available_inputs(sk);
                let initial_available_inputs_count = available_inputs.len();

                let change_address = data_mapper.map_address(Some((pk, Some(sk))))?;
                algorithm.set_available_utxos(available_inputs)?;
                let first_stage_select_result = algorithm.select_inputs(
                    &mut estimate,
                    InputOutputSetup::<UTxODetails, UTxOBuilder>::from_fixed_inputs_and_outputs(
                        vec![],
                        non_change_outputs.clone(),
                        Some(change_address.clone()),
                    ),
                );

                let mut first_stage_select_result = match first_stage_select_result {
                    Ok(r) => r,
                    Err(err) => {
                        tracing::error!(
                            "initial selection didn't converge: {}, tx_number: {}, sk: {}",
                            err,
                            tx_number,
                            sk
                        );
                        selection_eligibility_criteria
                            .clone()
                            .borrow_mut()
                            .mark_key_as_insolvent(sk);
                        remove_inputs_from_consideration(
                            inputs,
                            &mut utxo_accumulator,
                            &mut actual_balance_acc,
                            &mut computed_balance_acc,
                            selection_eligibility_criteria.clone(),
                        );
                        add_balances_from_partial_outputs(
                            tx_number as u64,
                            outputs,
                            &mut utxo_accumulator,
                            &mut computed_balance_acc,
                            &mut data_mapper,
                        )?;
                        continue;
                    }
                };

                let mut available_inputs = algorithm.get_available_utxos()?;

                let initial_fixed_outputs = first_stage_select_result.fixed_outputs.clone();

                let mut selected_changes = first_stage_select_result.changes.clone();
                let mut selected_inputs = first_stage_select_result.chosen_inputs.clone();
                let mut fee_computed = first_stage_select_result.fee.clone();

                assert_eq!(
                    selected_inputs.len() + available_inputs.len(),
                    initial_available_inputs_count
                );

                if !first_stage_select_result.are_utxos_balanced() && allow_balance_change {
                    balance_change_algo.set_available_utxos(available_inputs.clone())?;

                    // now all selected inputs are chosen ones
                    let mut fixed_inputs = first_stage_select_result.fixed_inputs;
                    fixed_inputs.append(&mut first_stage_select_result.chosen_inputs);

                    // outputs as well
                    let mut fixed_outputs = first_stage_select_result.fixed_outputs;
                    fixed_outputs.append(&mut first_stage_select_result.changes);

                    let second_stage_select_result = balance_change_algo.select_inputs(
                        &mut estimate,
                        InputOutputSetup::from_fixed_inputs_and_outputs(
                            fixed_inputs,
                            fixed_outputs,
                            Some(change_address.clone()),
                        ),
                    );

                    let mut second_stage_select_result = match second_stage_select_result {
                        Ok(r) if r.are_utxos_balanced() => r,
                        _ => {
                            if let Err(err) = second_stage_select_result {
                                tracing::error!("balance change selection didn't converge: {}, tx_number: {}, sk: {}", err, tx_number, sk);
                            } else {
                                tracing::error!("balance change selection didn't converge: utxos are not balanced, tx_number: {}, sk: {}", tx_number, sk);
                            }
                            selection_eligibility_criteria
                                .clone()
                                .borrow_mut()
                                .mark_key_as_insolvent(sk);
                            remove_inputs_from_consideration(
                                inputs,
                                &mut utxo_accumulator,
                                &mut actual_balance_acc,
                                &mut computed_balance_acc,
                                selection_eligibility_criteria.clone(),
                            );
                            add_balances_from_partial_outputs(
                                tx_number as u64,
                                outputs,
                                &mut utxo_accumulator,
                                &mut computed_balance_acc,
                                &mut data_mapper,
                            )?;
                            continue;
                        }
                    };

                    // changes from first stage + changes from balance + original fixed outputs = all outputs
                    available_inputs = balance_change_algo.get_available_utxos()?;

                    selected_changes.append(&mut second_stage_select_result.changes);
                    selected_inputs.append(&mut second_stage_select_result.chosen_inputs);

                    fee_computed = second_stage_select_result.fee;
                } else if !first_stage_select_result.are_utxos_balanced() {
                    tracing::error!("initial selection didn't converge and balance change is switched off, tx_number: {}, sk: {}", tx_number, sk);
                    tracing::error!(
                        "input balance: {:?}, output balance: {:?}, fee: {:?}",
                        first_stage_select_result.input_balance,
                        first_stage_select_result.output_balance,
                        first_stage_select_result.fee
                    );
                    tracing::error!("selected inputs:");
                    for output in first_stage_select_result.chosen_inputs.iter() {
                        tracing::error!("selected: {:?}", output);
                    }
                    tracing::error!("fixed outputs:");
                    for output in first_stage_select_result.fixed_outputs.iter() {
                        tracing::error!("output: {:?}", output);
                    }
                    tracing::error!("change outputs:");
                    for output in first_stage_select_result.changes.iter() {
                        tracing::error!("change: {:?}", output);
                    }
                    selection_eligibility_criteria
                        .clone()
                        .borrow_mut()
                        .mark_key_as_insolvent(sk);
                    remove_inputs_from_consideration(
                        inputs,
                        &mut utxo_accumulator,
                        &mut actual_balance_acc,
                        &mut computed_balance_acc,
                        selection_eligibility_criteria.clone(),
                    );
                    add_balances_from_partial_outputs(
                        tx_number as u64,
                        outputs,
                        &mut utxo_accumulator,
                        &mut computed_balance_acc,
                        &mut data_mapper,
                    )?;
                    continue;
                }

                assert_eq!(
                    available_inputs.len() + selected_inputs.len(),
                    initial_available_inputs_count
                );

                utxo_accumulator.set_available_inputs(sk, available_inputs);
                utxo_accumulator.add_from_outputs(
                    builders_to_utxo_details(
                        tx_number as u64,
                        initial_fixed_outputs
                            .iter()
                            .cloned()
                            .chain(selected_changes.iter().cloned())
                            .collect(),
                    )?,
                    &mut data_mapper,
                )?;

                computed_balance_acc
                    .reduce_balance_from_utxos(&selected_inputs, &mut data_mapper)?;
                computed_balance_acc.add_balance_from_builders(
                    &initial_fixed_outputs
                        .iter()
                        .cloned()
                        .chain(selected_changes.iter().cloned())
                        .collect::<Vec<_>>(),
                    &mut data_mapper,
                )?;

                computed_balance_acc.add_fee_spending(sk, &fee_computed);
            }
            TxEvent::Partial { to } => {
                actual_balance_acc.add_balance_from(&to, &mut data_mapper)?;
                add_balances_from_partial_outputs(
                    tx_number as u64,
                    to,
                    &mut utxo_accumulator,
                    &mut computed_balance_acc,
                    &mut data_mapper,
                )?;
            }
        }

        if tx_number % 1000 == 0 {
            tracing::info!("Processed line {:?}", tx_number);
        }
    }

    for stake_key in selection_eligibility_criteria
        .as_ref()
        .borrow()
        .get_whitelisted_non_banned()
        .iter()
    {
        collect_stats(
            stake_key,
            read,
            &paths,
            &actual_balance_acc,
            &computed_balance_acc,
            &utxo_accumulator,
            &mut balance_points_acc,
            &mut utxo_count_acc,
        );
    }

    tracing::info!(
        "Total converged addresses: {:?}",
        computed_balance_acc.len()
    );
    tracing::info!(
        "Total insolvent addresses: {:?}",
        selection_eligibility_criteria
            .as_ref()
            .borrow()
            .total_insolvent_addresses()
    );
    tracing::info!(
        "Total banned addresses: {:?}",
        selection_eligibility_criteria
            .as_ref()
            .borrow()
            .total_banned_addresses()
    );

    selection_eligibility_criteria
        .borrow_mut()
        .print_banned(paths.output_discarded)?;
    selection_eligibility_criteria
        .borrow_mut()
        .print_insolvent(paths.output_insolvent)?;

    print_balances(
        actual_balance_acc,
        computed_balance_acc,
        paths.output_balance,
        paths.output_balance_short,
    )?;

    if let Some(path) = paths.utxos_path {
        utxo_accumulator.print_utxos(path)?;
    }

    if let Some(path) = paths.balance_points_path {
        balance_points_acc.dump_stats(
            path,
            "ada_computed,ada_actual,fee_computed,fee_actual".to_string(),
        )?;
    }

    if let Some(path) = paths.utxos_balance_path {
        utxo_count_acc.dump_stats(path, "utxo_count".to_string())?;
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn collect_stats(
    stake_key: &u64,
    tx_number: u64,
    paths: &PathsConfig,
    actual_balance_acc: &BalanceAccumulator,
    computed_balance_acc: &BalanceAccumulator,
    utxo_accumulator: &UTxOStoreAccumulator,
    balance_points_acc: &mut StatsAccumulator<BalanceStats>,
    utxo_count_acc: &mut StatsAccumulator<u64>,
) {
    if paths.balance_points_path.is_some() {
        balance_points_acc.add_stats(
            *stake_key,
            tx_number,
            BalanceStats {
                ada_computed: balance_to_i64(
                    computed_balance_acc.get_balance(*stake_key, TokenId::MAIN),
                ),
                ada_actual: balance_to_i64(
                    actual_balance_acc.get_balance(*stake_key, TokenId::MAIN),
                ),
                fee_computed: i64::from_str(
                    computed_balance_acc
                        .get_fee(*stake_key)
                        .to_string()
                        .as_str(),
                )
                .unwrap(),
                fee_actual: i64::from_str(
                    actual_balance_acc.get_fee(*stake_key).to_string().as_str(),
                )
                .unwrap(),
            },
        );
    }
    if paths.utxos_balance_path.is_some() {
        utxo_count_acc.add_stats(
            *stake_key,
            tx_number,
            utxo_accumulator.get_available_inputs(*stake_key).len() as u64,
        );
    }
}

fn remove_inputs_from_consideration(
    inputs: Vec<TxOutput>,
    utxo_accumulator: &mut UTxOStoreAccumulator,
    actual_balance_acc: &mut BalanceAccumulator,
    computed_balance_acc: &mut BalanceAccumulator,
    selection_eligibility_criteria: Rc<RefCell<SelectionEligibility>>,
) {
    for input in inputs.into_iter() {
        if let Some((_, Some(sk))) = input.address {
            utxo_accumulator.remove_stake_key(sk);
            actual_balance_acc.remove_stake_key(sk);
            computed_balance_acc.remove_stake_key(sk);
            selection_eligibility_criteria
                .clone()
                .borrow_mut()
                .mark_key_as_insolvent(sk);
        }
    }
}

fn add_balances_from_partial_outputs<DataMapper: CardanoDataMapper>(
    tx_number: u64,
    outputs: Vec<TxOutput>,
    utxo_accumulator: &mut UTxOStoreAccumulator,
    computed_balance_acc: &mut BalanceAccumulator,
    data_mapper: &mut DataMapper,
) -> anyhow::Result<()> {
    computed_balance_acc.add_balance_from(&outputs, data_mapper)?;
    let builders = tx_outputs_to_utxo_builders(outputs, data_mapper)?;
    let outputs = builders_to_utxo_details(tx_number, builders)?;
    utxo_accumulator.add_from_outputs(outputs, data_mapper)?;
    Ok(())
}

fn print_balances(
    actual_balance_acc: BalanceAccumulator,
    computed_balance_acc: BalanceAccumulator,
    output_balance: PathBuf,
    output_balance_short: PathBuf,
) -> anyhow::Result<()> {
    let mut output_balance = File::create(output_balance)?;
    let mut output_balance_short = File::create(output_balance_short)?;

    let (computed_balances, computed_fee) = computed_balance_acc.to_balances_and_fee();
    let (actual_balances, actual_fee) = actual_balance_acc.to_balances_and_fee();

    let keys = computed_balances.iter();

    let mut better_than_actual: u64 = 0;
    let mut not_worse_than_actual: u64 = 0;
    let mut worse_than_actual: u64 = 0;

    let mut non_checkable: u64 = 0;

    let mut not_found_actual: u64 = 0;
    let mut not_found_token_actual: u64 = 0;

    for (key, computed) in keys {
        let actual = if let Some(balance) = actual_balances.get(key) {
            balance
        } else {
            not_found_actual += 1;
            output_balance.write_all(format!("no actual data: address: {key:?}\n").as_bytes())?;
            continue;
        };
        let mut better_than_actual_element_wise = vec![];

        for (token, computed_token_balance) in computed.iter() {
            let actual_token_balance = match actual.get(token) {
                None => {
                    not_found_token_actual += 1;
                    output_balance.write_all(
                        format!("no token actual data: address: {key:?}, token: {token:?}\n")
                            .as_bytes(),
                    )?;
                    continue;
                }
                Some(b) => b,
            };
            let diff = match actual_token_balance {
                Balance::Debt(value) => computed_token_balance + value,
                Balance::Balanced => {
                    computed_token_balance + &dcspark_core::Value::<Regulated>::zero()
                }
                Balance::Excess(value) => computed_token_balance - value,
            };
            let print_value = match diff {
                Balance::Debt(value) => {
                    better_than_actual_element_wise.push(1);
                    format!("worse: -{value}")
                }
                Balance::Balanced => {
                    better_than_actual_element_wise.push(0);
                    format!("same: {}", dcspark_core::Value::<Regulated>::zero())
                }
                Balance::Excess(value) => {
                    better_than_actual_element_wise.push(-1);
                    format!("better: {value}")
                }
            };
            output_balance.write_all(
                format!(
                    "diff: address: {:?}, token: {:?}, diff: {:?}, actual: {:?}, computed: {:?}, fee actual: {:?}, fee computed: {:?}\n",
                    key, token, print_value, actual_token_balance, computed_token_balance, actual_fee.get(key), computed_fee.get(key),
                )
                .as_bytes(),
            )?;
        }
        if better_than_actual_element_wise.iter().all(|b| *b == 1) {
            better_than_actual += 1;
        } else if better_than_actual_element_wise.iter().all(|b| *b == -1) {
            worse_than_actual += 1;
        } else if better_than_actual_element_wise.iter().all(|b| *b >= 0) {
            not_worse_than_actual += 1;
        } else {
            non_checkable += 1;
        }
    }

    output_balance_short
        .write_all(format!("better than actual: {better_than_actual:?}\n").as_bytes())?;
    output_balance_short
        .write_all(format!("not worse as actual: {not_worse_than_actual:?}\n").as_bytes())?;
    output_balance_short
        .write_all(format!("worse than actual: {worse_than_actual:?}\n").as_bytes())?;
    output_balance_short.write_all(format!("can't compare: {non_checkable:?}\n").as_bytes())?;
    output_balance_short
        .write_all(format!("not found actual: {not_found_actual:?}\n").as_bytes())?;
    output_balance_short
        .write_all(format!("not found token actual: {not_found_token_actual:?}\n").as_bytes())?;

    Ok(())
}

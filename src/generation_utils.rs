use std::path::PathBuf;

use anyhow::anyhow;
use cardano_multiplatform_lib::address::StakeCredential;

use crate::mapper::DataMapper;
use crate::tx_event::{TxAsset, TxEvent, TxOutput};
use cardano_multiplatform_lib::PolicyID;

use dcspark_core::Regulated;

use entity::prelude::TransactionModel;

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader, Write};

#[allow(clippy::too_many_arguments)]
pub fn carp_tx_to_events(
    tx: &TransactionModel,
    previous_outputs: &mut HashMap<String, HashMap<u64, TxOutput>>,
    stake_address_to_num: &mut DataMapper<StakeCredential>,
    payment_address_to_num: &mut DataMapper<StakeCredential>,
    policy_id_to_num: &mut DataMapper<PolicyID>,
    asset_name_to_num: &mut DataMapper<String>,
    banned_addresses: &mut HashSet<(u64, Option<u64>)>,
    unparsed_transactions: &mut Vec<TransactionModel>,
) -> anyhow::Result<Option<TxEvent>> {
    let payload: &Vec<u8> = &tx.payload;
    let tx_hash = hex::encode(tx.hash.clone());
    match cardano_multiplatform_lib::Transaction::from_bytes(payload.clone()) {
        Ok(parsed) => {
            let body = parsed.body();
            // inputs handle
            let inputs = body.inputs();

            let (has_banned_addresses, input_events) = match get_input_intents(
                &tx_hash,
                tx.id as u64,
                inputs,
                previous_outputs,
                banned_addresses,
            ) {
                Ok(output) => output,
                Err(err) => {
                    tracing::warn!("error occurred while trying to get inputs: {:?}", err);
                    unparsed_transactions.push(tx.clone());
                    return Ok(None);
                }
            };

            if has_banned_addresses {
                ban_addresses_for_events(&input_events, banned_addresses)?;
            }

            // outputs handle
            let outputs = body.outputs();
            let output_events = match get_output_intents(
                &tx_hash,
                outputs,
                previous_outputs,
                payment_address_to_num,
                stake_address_to_num,
                policy_id_to_num,
                asset_name_to_num,
            ) {
                Ok(result) => result,
                Err(err) => {
                    tracing::warn!("error occurred while trying to get outputs: {:?}", err);
                    unparsed_transactions.push(tx.clone());
                    for input in input_events.into_iter() {
                        if let Some(addr) = input.address {
                            banned_addresses.insert(addr);
                        }
                    }
                    return Ok(None);
                }
            };

            let event = if has_banned_addresses {
                let output_events: Vec<TxOutput> = output_events
                    .into_iter()
                    .filter(|output| !output.is_byron() && !output.is_banned(banned_addresses))
                    .collect();
                if output_events.is_empty() {
                    None
                } else {
                    Some(TxEvent::Partial { to: output_events })
                }
            } else {
                Some(TxEvent::Full {
                    to: output_events,
                    fee: dcspark_core::Value::<Regulated>::from(u64::from(body.fee())),
                    from: input_events,
                })
            };

            if let Some(event) = event {
                match &event {
                    TxEvent::Full { to, fee, from } => {
                        let mut input_value = dcspark_core::Value::zero();
                        let mut output_value = dcspark_core::Value::zero();
                        for to in to.iter() {
                            output_value += &to.value;
                        }
                        output_value += fee;
                        for from in from.iter() {
                            input_value += &from.value;
                        }
                        if input_value != output_value {
                            for input in from.iter() {
                                if let Some(addr) = input.address {
                                    banned_addresses.insert(addr);
                                }
                            }
                            for output in to.iter() {
                                if let Some(addr) = output.address {
                                    banned_addresses.insert(addr);
                                }
                            }
                            return Ok(None);
                        }
                    }
                    TxEvent::Partial { .. } => {}
                }
                return Ok(Some(event));
            }
        }
        Err(err) => {
            tracing::warn!("Can't parse tx: {:?}, err: {:?}", tx_hash.clone(), err);
            unparsed_transactions.push(tx.clone());
        }
    }
    Ok(None)
}

pub fn dump_unparsed_transactions_to_file(
    path: PathBuf,
    txs: Vec<TransactionModel>,
) -> anyhow::Result<()> {
    let mut output = File::create(path)?;
    output.write_all(format!("{}\n", txs.len()).as_bytes())?;
    for tx in txs.into_iter() {
        output.write_all(format!("{}\n", serde_json::to_string(&tx)?).as_bytes())?;
    }
    Ok(())
}

pub fn clean_events(
    events_output_path: PathBuf,
    cleaned_events_output_path: PathBuf,
    banned_addresses: &HashSet<(u64, Option<u64>)>,
) -> anyhow::Result<()> {
    let file = File::open(events_output_path)?;
    let mut cleaned_file = File::create(cleaned_events_output_path)?;

    let reader = BufReader::new(file);
    let lines = reader.lines();
    for (num, line) in lines.enumerate() {
        let event: TxEvent = serde_json::from_str(line?.as_str())?;
        let event = match event {
            TxEvent::Partial { to } => {
                let to: Vec<TxOutput> = to
                    .into_iter()
                    .filter(|output| !output.is_byron() && !output.is_banned(banned_addresses))
                    .collect();
                if !to.is_empty() {
                    Some(TxEvent::Partial { to })
                } else {
                    None
                }
            }
            TxEvent::Full { to, fee, from } => {
                if from
                    .iter()
                    .any(|input| input.is_byron() || input.is_banned(banned_addresses))
                {
                    let new_to: Vec<TxOutput> = to
                        .into_iter()
                        .filter(|output| !output.is_byron() && !output.is_banned(banned_addresses))
                        .collect();
                    if !new_to.is_empty() {
                        Some(TxEvent::Partial { to: new_to })
                    } else {
                        None
                    }
                } else {
                    let new_to: Vec<TxOutput> = to
                        .into_iter()
                        .map(|mut output| {
                            if output.is_banned(banned_addresses) {
                                output.address = None;
                            }
                            output
                        })
                        .collect();
                    Some(TxEvent::Full {
                        to: new_to,
                        fee,
                        from,
                    })
                }
            }
        };
        if let Some(event) = event {
            cleaned_file.write_all(format!("{}\n", serde_json::to_string(&event)?).as_bytes())?;
        }
        if num % 100000 == 0 {
            tracing::info!("Processed {:?} entries", num + 1);
        }
    }

    Ok(())
}

fn get_input_intents(
    tx_hash: &String,
    tx_id: u64,
    inputs: cardano_multiplatform_lib::TransactionInputs,
    previous_outputs: &mut HashMap<String, HashMap<u64, TxOutput>>,
    banned_addresses: &HashSet<(u64, Option<u64>)>,
) -> anyhow::Result<(bool, Vec<TxOutput>)> {
    let mut has_byron_inputs = false;

    // try to parse input addresses and put in the set
    let mut parsed_inputs = Vec::new();
    let mut inputs_pointers = HashSet::<(String, u64)>::new();
    let mut seen_tx_ids = Vec::new();

    for input_index in 0..inputs.len() {
        let input = inputs.get(input_index);
        let input_tx_id = input.transaction_id().to_hex();
        let input_tx_index = u64::from(input.index());

        // try to find output that is now used as an input
        if let Some(outputs) = &mut previous_outputs.get_mut(&input_tx_id) {
            // we remove the spent input from the list
            if let Some(output) = outputs.remove(&input_tx_index) {
                inputs_pointers.insert((input_tx_id.clone(), input_tx_index));
                parsed_inputs.push(output);
            } else {
                if inputs_pointers.contains(&(input_tx_id.clone(), input_tx_index)) {
                    tracing::info!("Found tx using same output as an input multiple times: {:?}@{:?}, current tx: {:?}, id: {:?}",
                        input_tx_id,
                        input_tx_index,
                        tx_hash,
                        tx_id,
                    );
                    continue;
                }
                // invalid transaction
                tracing::warn!(
                    "Can't find matching output for used input: {:?}@{:?}, current tx: {:?}, id: {:?}",
                    input_tx_id,
                    input_tx_index,
                    tx_hash,
                    tx_id,
                );
                return Err(anyhow!(
                    "Can't find matching output for used input: {:?}@{:?}, current tx: {:?}, id: {:?}",
                    input.transaction_id().to_hex(),
                    input.index(),
                    tx_hash,
                    tx_id,
                ));
            }
        } else {
            has_byron_inputs = true; // might be byron address or sth
        }

        seen_tx_ids.push(input_tx_id);
    }

    for seen_id in seen_tx_ids {
        if previous_outputs
            .get(&seen_id)
            .map(|outputs| outputs.is_empty())
            .unwrap_or(false)
        {
            previous_outputs.remove(&seen_id);
        }
    }

    let has_banned_addresses = parsed_inputs.iter().any(|input| {
        input.address.is_none()
            || (input.address.is_some() && banned_addresses.contains(&input.address.unwrap()))
    });

    Ok((has_byron_inputs || has_banned_addresses, parsed_inputs))
}

fn get_output_intents(
    tx_hash: &str,
    outputs: cardano_multiplatform_lib::TransactionOutputs,
    previous_outputs: &mut HashMap<String, HashMap<u64, TxOutput>>,
    payment_address_mapping: &mut DataMapper<StakeCredential>,
    stake_address_mapping: &mut DataMapper<StakeCredential>,
    policy_to_num: &mut DataMapper<PolicyID>,
    asset_name_to_num: &mut DataMapper<String>,
) -> anyhow::Result<Vec<TxOutput>> {
    let mut parsed_outputs = Vec::new();
    for output_index in 0..outputs.len() {
        let output = outputs.get(output_index);

        let address = output.address();
        let address = match address.payment_cred() {
            None => {
                // this is byron output
                None
            }
            Some(payment) => {
                let payment_mapping = payment_address_mapping.add_if_not_presented(payment);
                let staking_mapping = address
                    .staking_cred()
                    .map(|staking| stake_address_mapping.add_if_not_presented(staking));
                Some((payment_mapping, staking_mapping))
            }
        };

        let amount = output.amount();
        let value = dcspark_core::Value::<Regulated>::from(u64::from(amount.coin()));
        let mut assets = Vec::new();

        if let Some(multiasset) = amount.multiasset() {
            let policy_ids = multiasset.keys();
            for policy_id_index in 0..policy_ids.len() {
                let policy_id = policy_ids.get(policy_id_index);
                if let Some(assets_by_policy_id) = multiasset.get(&policy_id) {
                    let asset_names = assets_by_policy_id.keys();
                    for asset_name_id in 0..asset_names.len() {
                        let asset_name = asset_names.get(asset_name_id);
                        let asset_value = assets_by_policy_id.get(&asset_name);
                        if let Some(asset_value) = asset_value {
                            let policy_mapping =
                                policy_to_num.add_if_not_presented(policy_id.clone());
                            let asset_name_mapping = asset_name_to_num
                                .add_if_not_presented(hex::encode(asset_name.name()));
                            assets.push(TxAsset {
                                asset_id: (policy_mapping, asset_name_mapping),
                                value: dcspark_core::Value::<Regulated>::from(u64::from(
                                    asset_value,
                                )),
                            })
                        }
                    }
                }
            }
        }

        parsed_outputs.push(TxOutput {
            address,
            value,
            assets,
        })
    }

    let entry = previous_outputs.entry(tx_hash.to_owned()).or_default();
    for (output_index, parsed_output) in parsed_outputs.iter().enumerate() {
        entry.insert(output_index as u64, parsed_output.clone());
    }

    Ok(parsed_outputs)
}

fn ban_addresses_for_events(
    events: &[TxOutput],
    banned_addresses: &mut HashSet<(u64, Option<u64>)>,
) -> anyhow::Result<()> {
    for event in events.iter() {
        if let Some((payment, staking)) = event.address {
            banned_addresses.insert((payment, staking));
        }
    }
    Ok(())
}

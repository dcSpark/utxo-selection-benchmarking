use crate::bench_utils::address_mapper::CardanoDataMapper;
use crate::tx_event::TxOutput;
use anyhow::anyhow;
use dcspark_core::tx::{TransactionAsset, TransactionId, UTxOBuilder, UTxODetails, UtxoPointer};
use dcspark_core::OutputIndex;
use std::sync::Arc;

fn tx_output_to_utxo_builder<DataMapper: CardanoDataMapper>(
    output: TxOutput,
    data_mapper: &mut DataMapper,
) -> anyhow::Result<UTxOBuilder> {
    let address = data_mapper.map_address(output.address)?;
    let mut assets = vec![];
    for asset in output.assets.into_iter() {
        let (policy_id, asset_name) = asset.asset_id;
        let policy_id = data_mapper.map_policy_id(policy_id)?;
        let asset_name = data_mapper.map_asset_name(asset_name)?;
        let fingerprint =
            data_mapper.map_policy_id_and_asset(policy_id.clone(), asset_name.clone())?;

        assets.push(TransactionAsset {
            policy_id,
            asset_name,
            fingerprint,
            quantity: asset.value,
        });
    }

    Ok(UTxOBuilder::new(address, output.value, assets))
}

pub fn tx_outputs_to_utxo_builders<DataMapper: CardanoDataMapper>(
    outputs: Vec<TxOutput>,
    data_mapper: &mut DataMapper,
) -> anyhow::Result<Vec<UTxOBuilder>> {
    let mut new_outputs = vec![];
    for output in outputs.into_iter() {
        new_outputs.push(tx_output_to_utxo_builder(output, data_mapper)?);
    }
    Ok(new_outputs)
}

pub fn builders_to_utxo_details(
    tx_number: u64,
    outputs: Vec<UTxOBuilder>,
) -> anyhow::Result<Vec<UTxODetails>> {
    let mut new_outputs = vec![];
    for (output_index, builder) in outputs.into_iter().enumerate() {
        let transaction_id = {
            let format = tx_number;
            let format = format.to_be_bytes().to_vec();
            let format_len = format.len();
            let mut bytes = vec![0u8; 32];
            for (index, item) in format.into_iter().enumerate() {
                bytes[32 - format_len + index] = item;
            }

            cardano_multiplatform_lib::crypto::TransactionHash::from_bytes(bytes)
                .map_err(|err| anyhow!("can't create tx hash: {}", err))?
        };

        new_outputs.push(UTxODetails {
            pointer: UtxoPointer {
                transaction_id: TransactionId::new(transaction_id.to_string()),
                output_index: OutputIndex::new(output_index as u64),
            },
            address: builder.address,
            value: builder.value,
            assets: builder.assets,
            metadata: Arc::new(Default::default()),
            extra: None,
        })
    }

    Ok(new_outputs)
}

pub fn utxos_to_builders(outputs: &[UTxODetails]) -> Vec<UTxOBuilder> {
    outputs
        .iter()
        .map(|utxo| UTxOBuilder {
            address: utxo.address.clone(),
            value: utxo.value.clone(),
            assets: utxo.assets.clone(),
            extra: None,
        })
        .collect::<Vec<_>>()
}

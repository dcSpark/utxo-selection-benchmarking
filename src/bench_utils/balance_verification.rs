use crate::tx_event::TxOutput;
use anyhow::anyhow;

use dcspark_core::{Balance, Regulated, Value};
use itertools::Itertools;
use std::collections::HashMap;

pub fn verify_io_balance(
    inputs: &[TxOutput],
    outputs: &[TxOutput],
    fee: &Value<Regulated>,
) -> anyhow::Result<()> {
    let mut balance = Balance::<Regulated>::zero();

    let mut asset_balance = HashMap::<(u64, u64), Balance<Regulated>>::new();

    for input in inputs.iter() {
        balance += &input.value;
        for asset in input.assets.iter() {
            *asset_balance.entry(asset.asset_id).or_default() += &asset.value;
        }
        if !input.assets.iter().map(|asset| asset.asset_id).all_unique() {
            return Err(anyhow!("found non unique asset in utxo: {:?}", input));
        }
    }

    for output in outputs.iter() {
        balance -= &output.value;
        for asset in output.assets.iter() {
            *asset_balance.entry(asset.asset_id).or_default() -= &asset.value;
        }
        if !output
            .assets
            .iter()
            .map(|asset| asset.asset_id)
            .all_unique()
        {
            return Err(anyhow!("found non unique asset in utxo: {:?}", output));
        }
    }

    balance -= fee;
    if !balance.balanced() {
        return Err(anyhow!("main asset is not balanced: balance {}", balance));
    }

    for (asset, balance) in asset_balance.iter() {
        if !balance.balanced() {
            return Err(anyhow!(
                "{:?} asset is not balanced: balance {}",
                asset,
                balance
            ));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::bench_utils::balance_verification::verify_io_balance;
    use crate::tx_event::{TxAsset, TxOutput};
    use dcspark_core::{Regulated, Value};

    fn correct_outputs() -> Vec<TxOutput> {
        vec![
            TxOutput {
                address: None,
                value: Value::from(1),
                assets: vec![TxAsset {
                    asset_id: (0, 0),
                    value: Value::from(110),
                }],
            },
            TxOutput {
                address: None,
                value: Value::from(100),
                assets: vec![],
            },
        ]
    }

    fn correct_inputs() -> Vec<TxOutput> {
        vec![
            TxOutput {
                address: None,
                value: Value::from(100),
                assets: vec![],
            },
            TxOutput {
                address: None,
                value: Value::from(1),
                assets: vec![TxAsset {
                    asset_id: (0, 0),
                    value: Value::from(100),
                }],
            },
            TxOutput {
                address: None,
                value: Value::from(1),
                assets: vec![TxAsset {
                    asset_id: (0, 0),
                    value: Value::from(10),
                }],
            },
        ]
    }

    #[test]
    fn verify_correct() {
        let inputs = correct_inputs();
        let outputs = correct_outputs();
        let fee = Value::<Regulated>::from(1);

        assert!(verify_io_balance(&inputs, &outputs, &fee).is_ok());
    }

    #[test]
    fn verify_incorrect_double_asset_input() {
        let inputs = vec![
            TxOutput {
                address: None,
                value: Value::from(100),
                assets: vec![],
            },
            TxOutput {
                address: None,
                value: Value::from(1),
                assets: vec![
                    TxAsset {
                        asset_id: (0, 0),
                        value: Value::from(100),
                    },
                    TxAsset {
                        asset_id: (0, 0),
                        value: Value::from(5),
                    },
                ],
            },
            TxOutput {
                address: None,
                value: Value::from(1),
                assets: vec![TxAsset {
                    asset_id: (0, 0),
                    value: Value::from(5),
                }],
            },
        ];
        let outputs = correct_outputs();
        let fee = Value::<Regulated>::from(1);

        let result = verify_io_balance(&inputs, &outputs, &fee);
        assert!(result.is_err());
        let error_string = result.err().unwrap().to_string();
        assert!(error_string.starts_with("found non unique asset in utxo"));
    }

    #[test]
    fn verify_incorrect_double_asset_output() {
        let inputs = correct_inputs();
        let outputs = vec![
            TxOutput {
                address: None,
                value: Value::from(1),
                assets: vec![
                    TxAsset {
                        asset_id: (0, 0),
                        value: Value::from(109),
                    },
                    TxAsset {
                        asset_id: (0, 0),
                        value: Value::from(1),
                    },
                ],
            },
            TxOutput {
                address: None,
                value: Value::from(100),
                assets: vec![],
            },
        ];
        let fee = Value::<Regulated>::from(1);

        let result = verify_io_balance(&inputs, &outputs, &fee);
        assert!(result.is_err());
        let error_string = result.err().unwrap().to_string();
        assert!(error_string.starts_with("found non unique asset in utxo"));
    }

    #[test]
    fn verify_incorrect_outputs_token_balance() {
        let inputs = correct_inputs();
        let outputs = vec![
            TxOutput {
                address: None,
                value: Value::from(1),
                assets: vec![TxAsset {
                    asset_id: (0, 0),
                    value: Value::from(109),
                }],
            },
            TxOutput {
                address: None,
                value: Value::from(100),
                assets: vec![],
            },
        ];
        let fee = Value::<Regulated>::from(1);

        let result = verify_io_balance(&inputs, &outputs, &fee);
        assert!(result.is_err());
        let error_string = result.err().unwrap().to_string();
        assert!(
            error_string.starts_with("(0, 0) asset is not balanced: balance"),
            "{}",
            error_string
        );
    }

    #[test]
    fn verify_incorrect_outputs_main_balance() {
        let inputs = correct_inputs();
        let outputs = vec![
            TxOutput {
                address: None,
                value: Value::from(1),
                assets: vec![TxAsset {
                    asset_id: (0, 0),
                    value: Value::from(110),
                }],
            },
            TxOutput {
                address: None,
                value: Value::from(99),
                assets: vec![],
            },
        ];
        let fee = Value::<Regulated>::from(1);

        let result = verify_io_balance(&inputs, &outputs, &fee);
        assert!(result.is_err());
        let error_string = result.err().unwrap().to_string();
        assert!(
            error_string.starts_with("main asset is not balanced: balance"),
            "{}",
            error_string
        );
    }
}

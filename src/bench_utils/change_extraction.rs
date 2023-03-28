use crate::tx_event::TxOutput;

pub struct OutputsStructure {
    pub changes: Vec<TxOutput>,
    pub fixed_outputs: Vec<TxOutput>,
}

pub fn extract_changes(outputs: &[TxOutput], keys: (u64, u64)) -> OutputsStructure {
    let mut changes: Vec<TxOutput> = vec![];
    let mut fixed_outputs: Vec<TxOutput> = vec![];

    let (change_pk, change_sk) = keys;

    for output in outputs.iter() {
        match output.address {
            Some((pk, Some(sk))) if pk == change_pk && sk == change_sk => {
                changes.push(output.clone());
            }
            _ => {
                fixed_outputs.push(output.clone());
            }
        }
    }

    OutputsStructure {
        changes,
        fixed_outputs,
    }
}

#[cfg(test)]
mod tests {
    use crate::bench_utils::change_extraction::extract_changes;
    use crate::tx_event::TxOutput;

    #[test]
    fn check_split_2_changes() {
        let outputs = vec![
            TxOutput {
                address: Some((3, Some(0))),
                value: Default::default(),
                assets: vec![],
            },
            TxOutput {
                address: Some((0, Some(1))),
                value: Default::default(),
                assets: vec![],
            },
            TxOutput {
                address: Some((1, Some(1))),
                value: Default::default(),
                assets: vec![],
            },
            TxOutput {
                address: Some((0, Some(1))),
                value: Default::default(),
                assets: vec![],
            },
        ];
        let result = extract_changes(&outputs, (0, 1));
        assert_eq!(
            result.changes,
            vec![
                TxOutput {
                    address: Some((0, Some(1))),
                    value: Default::default(),
                    assets: vec![]
                },
                TxOutput {
                    address: Some((0, Some(1))),
                    value: Default::default(),
                    assets: vec![]
                }
            ]
        );
        assert_eq!(
            result.fixed_outputs,
            vec![
                TxOutput {
                    address: Some((3, Some(0))),
                    value: Default::default(),
                    assets: vec![]
                },
                TxOutput {
                    address: Some((1, Some(1))),
                    value: Default::default(),
                    assets: vec![]
                }
            ]
        );
    }

    #[test]
    fn check_split_no_address() {
        let outputs = vec![TxOutput {
            address: Some((3, Some(0))),
            value: Default::default(),
            assets: vec![],
        }];
        let result = extract_changes(&outputs, (0, 1));
        assert_eq!(
            result.fixed_outputs,
            vec![TxOutput {
                address: Some((3, Some(0))),
                value: Default::default(),
                assets: vec![]
            },]
        );
        assert!(result.changes.is_empty());
    }
}

use crate::bench_utils::address_mapper::CardanoDataMapper;
use crate::bench_utils::selection_eligibility::SelectionEligibility;
use dcspark_core::tx::UTxODetails;
use dcspark_core::{Regulated, UTxOStore, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::rc::Rc;

pub struct UTxOAccumulator {
    utxos_by_stake_key: HashMap<u64, Vec<UTxODetails>>,
    criteria: Rc<RefCell<SelectionEligibility>>,
}

impl UTxOAccumulator {
    pub fn new(selection_eligibility: Rc<RefCell<SelectionEligibility>>) -> Self {
        Self {
            utxos_by_stake_key: Default::default(),
            criteria: selection_eligibility,
        }
    }

    pub fn get_available_inputs(&self, staking_key: u64) -> Vec<UTxODetails> {
        self.utxos_by_stake_key
            .get(&staking_key)
            .cloned()
            .unwrap_or_default()
    }

    pub fn set_available_inputs(&mut self, staking_key: u64, available_inputs: Vec<UTxODetails>) {
        let criteria = self.criteria.as_ref().borrow();
        if !criteria.is_banned(staking_key) && criteria.is_whitelisted(staking_key) {
            *self.utxos_by_stake_key.entry(staking_key).or_default() = available_inputs;
        } else {
            self.utxos_by_stake_key.remove(&staking_key);
        }
    }

    pub fn add_from_outputs<Mapper: CardanoDataMapper>(
        &mut self,
        outputs: Vec<UTxODetails>,
        mapper: &mut Mapper,
    ) -> anyhow::Result<()> {
        let criteria = self.criteria.as_ref().borrow();
        for output in outputs.iter() {
            let address = mapper.map_address_to_indices(output.address.clone())?;
            if let Some((_, Some(sk))) = address {
                if !criteria.is_banned(sk) && criteria.is_whitelisted(sk) {
                    self.utxos_by_stake_key
                        .entry(sk)
                        .or_default()
                        .push(output.clone());
                } else {
                    self.utxos_by_stake_key.remove(&sk);
                }
            }
        }

        Ok(())
    }

    pub fn remove_stake_key(&mut self, staking_key: u64) {
        self.utxos_by_stake_key.remove(&staking_key);
    }
}

pub struct UTxOStoreAccumulator {
    utxos_by_stake_key: HashMap<u64, UTxOStore>,
    criteria: Rc<RefCell<SelectionEligibility>>,
}

impl UTxOStoreAccumulator {
    pub fn new(selection_eligibility: Rc<RefCell<SelectionEligibility>>) -> Self {
        Self {
            utxos_by_stake_key: Default::default(),
            criteria: selection_eligibility,
        }
    }

    pub fn get_available_inputs(&self, staking_key: u64) -> UTxOStore {
        self.utxos_by_stake_key
            .get(&staking_key)
            .cloned()
            .unwrap_or_default()
    }

    pub fn set_available_inputs(&mut self, staking_key: u64, available_inputs: UTxOStore) {
        let criteria = self.criteria.as_ref().borrow();
        if !criteria.is_banned(staking_key) && criteria.is_whitelisted(staking_key) {
            *self.utxos_by_stake_key.entry(staking_key).or_default() = available_inputs;
        } else {
            self.utxos_by_stake_key.remove(&staking_key);
        }
    }

    pub fn add_from_outputs<Mapper: CardanoDataMapper>(
        &mut self,
        outputs: Vec<UTxODetails>,
        mapper: &mut Mapper,
    ) -> anyhow::Result<()> {
        let criteria = self.criteria.as_ref().borrow();
        for output in outputs.iter() {
            let address = mapper.map_address_to_indices(output.address.clone())?;
            if let Some((_, Some(sk))) = address {
                if !criteria.is_banned(sk) && criteria.is_whitelisted(sk) {
                    let mut mut_store = self.utxos_by_stake_key.entry(sk).or_default().thaw();
                    mut_store.insert(output.clone())?;
                    self.utxos_by_stake_key.insert(sk, mut_store.freeze());
                } else {
                    self.utxos_by_stake_key.remove(&sk);
                }
            }
        }

        Ok(())
    }

    pub fn remove_stake_key(&mut self, staking_key: u64) {
        self.utxos_by_stake_key.remove(&staking_key);
    }

    pub fn print_utxos(&self, path: PathBuf) -> anyhow::Result<()> {
        let mut file = File::create(path)?;

        for (sk, utxos) in self.utxos_by_stake_key.iter() {
            let criteria = self.criteria.as_ref().borrow();
            if criteria.is_whitelisted(*sk) && !criteria.is_banned(*sk) {
                assert_eq!(utxos.len(), utxos.iter_ordered_by_wmain().count());
                let mut greater_than_10 = 0;
                let mut less_than_10 = 0;
                for utxo in utxos.iter_ordered_by_wmain() {
                    file.write_all(format!("{}, {:?}\n", utxo.value, utxo.assets).as_bytes())?;
                    if utxo.value > Value::<Regulated>::from(10_000_000) {
                        greater_than_10 += 1;
                    } else {
                        less_than_10 += 1;
                    }
                }
                file.write_all(format!("total greater than 10: {}\n", greater_than_10).as_bytes())?;
                file.write_all(format!("total less than 10: {}\n", less_than_10).as_bytes())?;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::bench_utils::address_mapper::StringAddressMapper;
    use crate::bench_utils::selection_eligibility::SelectionEligibility;
    use crate::bench_utils::utxo_accumulator::UTxOAccumulator;
    use dcspark_core::tx::{TransactionId, UTxODetails, UtxoPointer};
    use dcspark_core::{Address, OutputIndex, Regulated, Value};
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Arc;

    fn generate_utxo(index: u64, value: Value<Regulated>, pk: u64, sk: u64) -> UTxODetails {
        UTxODetails {
            pointer: UtxoPointer {
                transaction_id: TransactionId::new("0"),
                output_index: OutputIndex::new(index),
            },
            address: Address::new(format!("{}:{}", pk, sk)),
            value,
            assets: vec![],
            metadata: Arc::new(Default::default()),
            extra: None,
        }
    }

    #[test]
    fn can_add_outputs() {
        let criteria = Rc::new(RefCell::new(SelectionEligibility::default()));
        criteria
            .as_ref()
            .borrow_mut()
            .set_staking_keys_of_interest(vec![1, 2]);

        let mut mapper = StringAddressMapper::default();

        let mut utxo_acc = UTxOAccumulator::new(criteria);
        utxo_acc.set_available_inputs(1, vec![generate_utxo(0, Value::from(100), 0, 1)]);
        assert_eq!(utxo_acc.get_available_inputs(1).len(), 1);
        assert_eq!(
            utxo_acc.get_available_inputs(1).first().unwrap().value,
            Value::from(100)
        );

        utxo_acc.set_available_inputs(0, vec![generate_utxo(1, Value::from(100), 0, 0)]);
        assert!(utxo_acc.get_available_inputs(0).is_empty());

        utxo_acc
            .add_from_outputs(
                vec![
                    generate_utxo(2, Value::from(100), 0, 0),
                    generate_utxo(3, Value::from(100), 0, 1),
                ],
                &mut mapper,
            )
            .unwrap();
        assert_eq!(utxo_acc.get_available_inputs(1).len(), 2);
        assert!(utxo_acc
            .get_available_inputs(1)
            .iter()
            .all(|item| vec![0u64, 3u64].contains(&u64::from(item.pointer.output_index))));

        utxo_acc.remove_stake_key(1);
        assert!(utxo_acc.get_available_inputs(1).is_empty());
    }

    #[test]
    fn external_change_works() {
        let criteria = Rc::new(RefCell::new(SelectionEligibility::default()));
        criteria
            .as_ref()
            .borrow_mut()
            .set_staking_keys_of_interest(vec![1, 2]);

        let mut mapper = StringAddressMapper::default();

        let mut utxo_acc = UTxOAccumulator::new(criteria.clone());
        utxo_acc.set_available_inputs(1, vec![generate_utxo(0, Value::from(100), 0, 1)]);
        assert_eq!(utxo_acc.get_available_inputs(1).len(), 1);
        assert_eq!(
            utxo_acc.get_available_inputs(1).first().unwrap().value,
            Value::from(100)
        );

        utxo_acc
            .add_from_outputs(
                vec![
                    generate_utxo(1, Value::from(100), 0, 0),
                    generate_utxo(2, Value::from(100), 0, 1),
                ],
                &mut mapper,
            )
            .unwrap();
        assert_eq!(utxo_acc.get_available_inputs(1).len(), 2);
        assert!(utxo_acc
            .get_available_inputs(1)
            .iter()
            .all(|item| vec![0u64, 2u64].contains(&u64::from(item.pointer.output_index))));

        criteria.as_ref().borrow_mut().ban_key(1);

        utxo_acc
            .add_from_outputs(vec![generate_utxo(3, Value::from(100), 0, 1)], &mut mapper)
            .unwrap();
        assert!(utxo_acc.get_available_inputs(1).is_empty());
    }
}

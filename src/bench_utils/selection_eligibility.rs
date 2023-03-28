use crate::tx_event::TxOutput;
use std::collections::HashSet;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

pub struct SelectionEligibility {
    insolvent_staking_keys: HashSet<u64>,
    banned_staking_keys: HashSet<u64>,

    staking_keys_of_interest: HashSet<u64>,
    allow_all_stake_keys: bool,
}

impl Default for SelectionEligibility {
    fn default() -> Self {
        Self {
            insolvent_staking_keys: Default::default(),
            banned_staking_keys: Default::default(),
            staking_keys_of_interest: Default::default(),
            allow_all_stake_keys: true,
        }
    }
}

impl SelectionEligibility {
    pub fn is_banned(&self, staking_key: u64) -> bool {
        self.banned_staking_keys.contains(&staking_key)
            || self.insolvent_staking_keys.contains(&staking_key)
    }

    pub fn is_whitelisted(&self, staking_key: u64) -> bool {
        self.staking_keys_of_interest.contains(&staking_key) || self.allow_all_stake_keys
    }

    /* we don't take txs:
     * - with byron inputs
     * - with more than one staking key in inputs
     * - with no staking key in inputs
     */
    pub fn should_perform_selection(&mut self, inputs: &[TxOutput]) -> Option<(Vec<u64>, u64)> {
        let mut seen_keys = HashSet::<u64>::new();
        let mut payment_keys: Vec<u64> = vec![];

        let mut should_ban = false;

        for input in inputs.iter() {
            match input.address {
                Some((pk, Some(sk))) => {
                    seen_keys.insert(sk);
                    payment_keys.push(pk);
                }
                _ => {
                    should_ban = true; // no staking key in input
                    break;
                }
            }
        }

        if seen_keys.len() != 1 {
            should_ban = true; // more than 1 key in input
        }

        let mut selected_sk: Option<(Vec<u64>, u64)> = None;

        if let Some(sk) = seen_keys.iter().next().cloned() {
            if self.is_banned(sk) {
                should_ban = true;
            } else if self.is_whitelisted(sk) {
                selected_sk = Some((payment_keys, sk));
            }
        } else {
            should_ban = true;
        }

        if should_ban {
            self.ban_keys_from_inputs(inputs);
            None
        } else {
            selected_sk
        }
    }

    pub fn set_staking_keys_of_interest(&mut self, keys: Vec<u64>) {
        keys.into_iter().for_each(|key| {
            let _ = self.staking_keys_of_interest.insert(key);
        });
        self.allow_all_stake_keys = false;
    }

    pub fn mark_key_as_insolvent(&mut self, staking_key: u64) {
        if self.staking_keys_of_interest.contains(&staking_key) {
            panic!("staking key of interest is insolvent");
        }
        self.insolvent_staking_keys.insert(staking_key);
    }

    pub fn ban_key(&mut self, staking_key: u64) {
        self.banned_staking_keys.insert(staking_key);
    }

    fn ban_keys_from_inputs(&mut self, inputs: &[TxOutput]) {
        for input in inputs.iter() {
            if let Some((_, Some(sk))) = input.address {
                self.banned_staking_keys.insert(sk);
            }
        }
    }

    pub fn print_banned(&self, path: PathBuf) -> anyhow::Result<()> {
        Self::print_hashmap(&self.banned_staking_keys, path)
    }

    pub fn print_insolvent(&self, path: PathBuf) -> anyhow::Result<()> {
        Self::print_hashmap(&self.insolvent_staking_keys, path)
    }

    fn print_hashmap(keys: &HashSet<u64>, path: PathBuf) -> anyhow::Result<()> {
        let mut file = File::create(path)?;
        for key in keys.iter() {
            file.write_all(format!("{key:?}\n").as_bytes())?;
        }
        Ok(())
    }

    pub fn total_banned_addresses(&self) -> usize {
        self.banned_staking_keys.len()
    }

    pub fn total_insolvent_addresses(&self) -> usize {
        self.insolvent_staking_keys.len()
    }
}

#[cfg(test)]
mod tests {
    use crate::bench_utils::selection_eligibility::SelectionEligibility;
    use crate::tx_event::TxOutput;

    #[test]
    fn no_preference_works() {
        let criteria = SelectionEligibility::default();
        assert!(criteria.is_whitelisted(0));
    }

    #[test]
    fn no_preference_doesnt_work_when_interest_specified() {
        let mut criteria = SelectionEligibility::default();
        criteria.set_staking_keys_of_interest(vec![1]);
        assert!(!criteria.is_whitelisted(0));
        assert!(criteria.is_whitelisted(1));
    }

    #[test]
    fn insolvent_doesnt_work() {
        let mut criteria = SelectionEligibility::default();
        criteria.set_staking_keys_of_interest(vec![1]);
        assert!(!criteria.is_whitelisted(0));
        assert!(criteria.is_whitelisted(1));
        assert!(!criteria.is_banned(1));
        criteria.mark_key_as_insolvent(1);
        assert!(criteria.is_banned(1));
        assert!(criteria.is_whitelisted(1));
    }

    #[test]
    fn discarded_doesnt_work() {
        let mut criteria = SelectionEligibility::default();
        criteria.set_staking_keys_of_interest(vec![1]);
        assert!(!criteria.is_whitelisted(0));
        assert!(criteria.is_whitelisted(1));
        assert!(!criteria.is_banned(1));
        criteria.ban_key(1);
        assert!(criteria.is_banned(1));
        assert!(criteria.is_whitelisted(1));
    }

    #[test]
    fn two_stake_keys_doesnt_work() {
        let mut criteria = SelectionEligibility::default();
        let keys = vec![0, 1];
        assert!(!keys.iter().map(|key| criteria.is_banned(*key)).any(|s| s));
        let result = criteria.should_perform_selection(
            &keys
                .iter()
                .map(|key| TxOutput {
                    address: Some((10, Some(*key))),
                    value: Default::default(),
                    assets: vec![],
                })
                .collect::<Vec<_>>(),
        );
        assert!(result.is_none());
        assert!(keys.iter().map(|key| criteria.is_banned(*key)).all(|s| s));
        let result = criteria.should_perform_selection(&[TxOutput {
            address: Some((11, Some(0))),
            value: Default::default(),
            assets: vec![],
        }]);
        assert!(result.is_none());
        assert!(keys.iter().map(|key| criteria.is_banned(*key)).all(|s| s));
    }

    #[test]
    fn two_payment_keys_work() {
        let mut criteria = SelectionEligibility::default();
        let keys = vec![2, 3];
        assert!(!keys.iter().map(|key| criteria.is_banned(*key)).any(|s| s));
        let result = criteria.should_perform_selection(
            &keys
                .iter()
                .map(|key| TxOutput {
                    address: Some((*key, Some(0))),
                    value: Default::default(),
                    assets: vec![],
                })
                .collect::<Vec<_>>(),
        );
        assert!(result.is_some());
        assert_eq!(result.unwrap(), (vec![2, 3], 0));
        assert!(!criteria.is_banned(0));
        let result = criteria.should_perform_selection(&[TxOutput {
            address: Some((0, Some(0))),
            value: Default::default(),
            assets: vec![],
        }]);

        assert!(result.is_some());
        assert_eq!(result.unwrap(), (vec![0], 0));
        assert!(!criteria.is_banned(0));
    }
}

use crate::bench_utils::address_mapper::CardanoDataMapper;
use crate::bench_utils::output_utils::utxos_to_builders;
use crate::bench_utils::selection_eligibility::SelectionEligibility;
use crate::tx_event::TxOutput;
use dcspark_core::tx::{UTxOBuilder, UTxODetails};
use dcspark_core::{Balance, Regulated, TokenId, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub struct BalanceAccumulator {
    balance_by_stake_key: HashMap<u64, HashMap<TokenId, Balance<Regulated>>>,
    fee_by_stake_key: HashMap<u64, Value<Regulated>>,
    criteria: Rc<RefCell<SelectionEligibility>>,
}

impl BalanceAccumulator {
    pub fn new(selection_eligibility: Rc<RefCell<SelectionEligibility>>) -> Self {
        Self {
            balance_by_stake_key: Default::default(),
            fee_by_stake_key: Default::default(),
            criteria: selection_eligibility,
        }
    }

    pub fn get_balance(&self, stake_key: u64, token: TokenId) -> Balance<Regulated> {
        self.balance_by_stake_key
            .get(&stake_key)
            .and_then(|map| map.get(&token))
            .cloned()
            .unwrap_or_default()
    }

    pub fn get_fee(&self, stake_key: u64) -> Value<Regulated> {
        self.fee_by_stake_key
            .get(&stake_key)
            .cloned()
            .unwrap_or_default()
    }

    pub fn reduce_balance_from<DataMapper: CardanoDataMapper>(
        &mut self,
        from: &[TxOutput],
        mapper: &mut DataMapper,
    ) -> anyhow::Result<()> {
        for builder in from.iter() {
            let sk = match builder.address {
                Some((_, Some(sk))) => sk,
                _ => continue,
            };
            let criteria = self.criteria.as_ref().borrow();
            if criteria.is_whitelisted(sk) && !criteria.is_banned(sk) {
                let entry = self.balance_by_stake_key.entry(sk).or_default();
                *entry.entry(TokenId::MAIN).or_default() -= &builder.value;
                for asset in builder.assets.iter() {
                    let fingerprint = mapper
                        .map_policy_id_and_asset_indices(asset.asset_id.0, asset.asset_id.1)?;
                    *entry.entry(fingerprint).or_default() -= &asset.value;
                }
            } else {
                self.balance_by_stake_key.remove(&sk);
            }
        }

        Ok(())
    }

    pub fn add_balance_from<DataMapper: CardanoDataMapper>(
        &mut self,
        from: &[TxOutput],
        mapper: &mut DataMapper,
    ) -> anyhow::Result<()> {
        for builder in from.iter() {
            let sk = match builder.address {
                Some((_, Some(sk))) => sk,
                _ => continue,
            };
            let criteria = self.criteria.as_ref().borrow();
            if criteria.is_whitelisted(sk) && !criteria.is_banned(sk) {
                let entry = self.balance_by_stake_key.entry(sk).or_default();
                *entry.entry(TokenId::MAIN).or_default() += &builder.value;
                for asset in builder.assets.iter() {
                    let fingerprint = mapper
                        .map_policy_id_and_asset_indices(asset.asset_id.0, asset.asset_id.1)?;
                    *entry.entry(fingerprint).or_default() += &asset.value;
                }
            } else {
                self.balance_by_stake_key.remove(&sk);
            }
        }

        Ok(())
    }

    pub fn reduce_balance_from_builders<DataMapper: CardanoDataMapper>(
        &mut self,
        from: &[UTxOBuilder],
        mapper: &mut DataMapper,
    ) -> anyhow::Result<()> {
        for builder in from.iter() {
            let sk = match mapper.map_address_to_indices(builder.address.clone())? {
                Some((_, Some(sk))) => sk,
                _ => continue,
            };
            let criteria = self.criteria.as_ref().borrow();
            if criteria.is_whitelisted(sk) && !criteria.is_banned(sk) {
                let entry = self.balance_by_stake_key.entry(sk).or_default();
                *entry.entry(TokenId::MAIN).or_default() -= &builder.value;
                for asset in builder.assets.iter() {
                    *entry.entry(asset.fingerprint.clone()).or_default() -= &asset.quantity;
                }
            } else {
                self.balance_by_stake_key.remove(&sk);
            }
        }

        Ok(())
    }

    pub fn add_balance_from_builders<DataMapper: CardanoDataMapper>(
        &mut self,
        from: &[UTxOBuilder],
        mapper: &mut DataMapper,
    ) -> anyhow::Result<()> {
        for builder in from.iter() {
            let sk = match mapper.map_address_to_indices(builder.address.clone())? {
                Some((_, Some(sk))) => sk,
                _ => continue,
            };
            let criteria = self.criteria.as_ref().borrow();
            if criteria.is_whitelisted(sk) && !criteria.is_banned(sk) {
                let entry = self.balance_by_stake_key.entry(sk).or_default();
                *entry.entry(TokenId::MAIN).or_default() += &builder.value;
                for asset in builder.assets.iter() {
                    *entry.entry(asset.fingerprint.clone()).or_default() += &asset.quantity;
                }
            } else {
                self.balance_by_stake_key.remove(&sk);
            }
        }

        Ok(())
    }

    pub fn reduce_balance_from_utxos<DataMapper: CardanoDataMapper>(
        &mut self,
        from: &[UTxODetails],
        mapper: &mut DataMapper,
    ) -> anyhow::Result<()> {
        let builders = utxos_to_builders(from);

        self.reduce_balance_from_builders(&builders, mapper)
    }

    pub fn add_balance_from_utxos<DataMapper: CardanoDataMapper>(
        &mut self,
        from: &[UTxODetails],
        mapper: &mut DataMapper,
    ) -> anyhow::Result<()> {
        let builders = utxos_to_builders(from);

        self.add_balance_from_builders(&builders, mapper)
    }

    pub fn add_fee_spending(&mut self, staking_key: u64, fee: &Value<Regulated>) {
        *self.fee_by_stake_key.entry(staking_key).or_default() += fee;
    }

    pub fn remove_stake_key(&mut self, staking_key: u64) {
        self.balance_by_stake_key.remove(&staking_key);
        self.fee_by_stake_key.remove(&staking_key);
    }

    pub fn len(&self) -> usize {
        self.balance_by_stake_key.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[allow(clippy::type_complexity)]
    pub fn to_balances_and_fee(
        self,
    ) -> (
        HashMap<u64, HashMap<TokenId, Balance<Regulated>>>,
        HashMap<u64, Value<Regulated>>,
    ) {
        (self.balance_by_stake_key, self.fee_by_stake_key)
    }
}

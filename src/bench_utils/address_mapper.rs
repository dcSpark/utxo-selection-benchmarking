use crate::mapper::DataMapper;
use anyhow::anyhow;
use cardano_multiplatform_lib::address::StakeCredential;
use cardano_multiplatform_lib::PolicyID;
use dcspark_core::{Address, AssetName, PolicyId, TokenId};
use std::collections::HashMap;
use std::path::PathBuf;
use std::str::FromStr;

pub trait CardanoDataMapper {
    fn map_address(&mut self, address: Option<(u64, Option<u64>)>) -> anyhow::Result<Address>;
    fn map_address_to_indices(
        &mut self,
        address: Address,
    ) -> anyhow::Result<Option<(u64, Option<u64>)>>;

    fn map_policy_id(&mut self, policy_id: u64) -> anyhow::Result<PolicyId>;
    fn map_policy_id_to_index(&mut self, policy_id: PolicyId) -> anyhow::Result<u64>;

    fn map_asset_name(&mut self, asset_name: u64) -> anyhow::Result<AssetName>;
    fn map_asset_name_to_index(&mut self, asset_name: AssetName) -> anyhow::Result<u64>;

    fn map_token_id(&mut self, token: TokenId) -> anyhow::Result<(PolicyId, AssetName)>;
    fn map_policy_id_and_asset(
        &mut self,
        policy: PolicyId,
        asset: AssetName,
    ) -> anyhow::Result<TokenId>;

    fn map_policy_id_and_asset_indices(
        &mut self,
        policy: u64,
        asset: u64,
    ) -> anyhow::Result<TokenId> {
        let policy_id = self.map_policy_id(policy)?;
        let asset_name = self.map_asset_name(asset)?;
        self.map_policy_id_and_asset(policy_id, asset_name)
    }
}

#[derive(Default)]
pub struct StringAddressMapper {}

impl StringAddressMapper {
    const EMPTY_ADDRESS: &'static str = "null";

    fn parse_address_part(part: Option<&str>) -> anyhow::Result<Option<u64>> {
        match part {
            Some(Self::EMPTY_ADDRESS) => Ok(None),
            Some(x) if x.chars().all(|c| c.is_numeric()) => Ok(Some(u64::from_str(x)?)),
            _ => Err(anyhow!("can't parse address: {:?}", part)),
        }
    }
}

impl CardanoDataMapper for StringAddressMapper {
    fn map_address(&mut self, address: Option<(u64, Option<u64>)>) -> anyhow::Result<Address> {
        let result = match address {
            None => Address::new(Self::EMPTY_ADDRESS),
            Some((pk, Some(sk))) => Address::new(format!("{}:{}", pk, sk)),
            Some((pk, None)) => Address::new(format!("{}:{}", pk, Self::EMPTY_ADDRESS)),
        };
        Ok(result)
    }

    fn map_address_to_indices(
        &mut self,
        address: Address,
    ) -> anyhow::Result<Option<(u64, Option<u64>)>> {
        let mut address_parts = address.split(':');
        let pk = match Self::parse_address_part(address_parts.next())? {
            None => return Ok(None),
            Some(pk) => pk,
        };

        let sk = Self::parse_address_part(address_parts.next())?;

        Ok(Some((pk, sk)))
    }

    fn map_token_id(&mut self, token: TokenId) -> anyhow::Result<(PolicyId, AssetName)> {
        let mut token_parts = token.as_ref().split(':');
        let policy = match token_parts.next() {
            Some(part) => u64::from_str(part)?,
            _ => return Err(anyhow!("no policy in the address")),
        };

        let asset_name = match token_parts.next() {
            Some(part) => u64::from_str(part)?,
            _ => return Err(anyhow!("no asset name in the address")),
        };

        Ok((
            PolicyId::new(policy.to_string()),
            AssetName::new(asset_name.to_string()),
        ))
    }

    fn map_policy_id_and_asset(
        &mut self,
        policy: PolicyId,
        asset: AssetName,
    ) -> anyhow::Result<TokenId> {
        Ok(TokenId::new(format!(
            "{}:{}",
            policy.as_ref(),
            asset.as_ref()
        )))
    }

    fn map_policy_id(&mut self, policy_id: u64) -> anyhow::Result<PolicyId> {
        Ok(PolicyId::new(policy_id.to_string()))
    }

    fn map_policy_id_to_index(&mut self, policy_id: PolicyId) -> anyhow::Result<u64> {
        Ok(u64::from_str(policy_id.as_ref())?)
    }

    fn map_asset_name(&mut self, asset_name: u64) -> anyhow::Result<AssetName> {
        Ok(AssetName::new(asset_name.to_string()))
    }

    fn map_asset_name_to_index(&mut self, asset_name: AssetName) -> anyhow::Result<u64> {
        Ok(u64::from_str(asset_name.as_ref())?)
    }
}

pub struct CardanoAddressMapper {
    payment_key_mapper: DataMapper<StakeCredential>,
    staking_key_mapper: DataMapper<StakeCredential>,
    policy_id_mapper: DataMapper<PolicyID>,
    asset_name_mapper: DataMapper<String>,
    network: u8,
    default_address: Address,
    mapped_tokens: HashMap<String, (PolicyId, AssetName)>,
}

impl CardanoAddressMapper {
    pub fn new(
        payment_key_path: PathBuf,
        staking_key_path: PathBuf,
        policy_id_path: PathBuf,
        asset_name_path: PathBuf,
        network: u8,
        default_address: Address,
    ) -> anyhow::Result<CardanoAddressMapper> {
        let payment_key_mapper = DataMapper::load_from_file(payment_key_path)?;
        let staking_key_mapper = DataMapper::load_from_file(staking_key_path)?;
        let policy_id_mapper = DataMapper::load_from_file(policy_id_path)?;
        let asset_name_mapper = DataMapper::load_from_file(asset_name_path)?;
        Ok(Self {
            payment_key_mapper,
            staking_key_mapper,
            policy_id_mapper,
            asset_name_mapper,
            network,
            default_address,
            mapped_tokens: Default::default(),
        })
    }
}

impl CardanoDataMapper for CardanoAddressMapper {
    fn map_address(&mut self, address: Option<(u64, Option<u64>)>) -> anyhow::Result<Address> {
        match address {
            None => Ok(self.default_address.clone()),
            Some((pk, Some(sk))) => {
                let pk = self
                    .payment_key_mapper
                    .get_by_index(pk)
                    .ok_or_else(|| anyhow!("can't find pk: {}", pk))?;
                let sk = self
                    .staking_key_mapper
                    .get_by_index(sk)
                    .ok_or_else(|| anyhow!("can't find sk: {}", sk))?;

                Ok(Address::new(
                    cardano_multiplatform_lib::address::BaseAddress::new(self.network, pk, sk)
                        .to_address()
                        .to_bech32(None)
                        .map_err(|err| anyhow!("can't convert address: {}", err))?,
                ))
            }
            Some((pk, None)) => {
                let pk = self
                    .payment_key_mapper
                    .get_by_index(pk)
                    .ok_or_else(|| anyhow!("can't find pk: {}", pk))?;

                Ok(Address::new(
                    cardano_multiplatform_lib::address::EnterpriseAddress::new(self.network, pk)
                        .to_address()
                        .to_bech32(None)
                        .map_err(|err| anyhow!("can't convert address: {}", err))?,
                ))
            }
        }
    }

    fn map_address_to_indices(
        &mut self,
        address: Address,
    ) -> anyhow::Result<Option<(u64, Option<u64>)>> {
        if address == self.default_address {
            return Ok(None);
        }
        let inner = cardano_multiplatform_lib::address::Address::from_bech32(address.as_ref())
            .map_err(|err| anyhow!("can't convert address: {}, err: {}", address, err))?;
        let pk = match inner.payment_cred() {
            None => return Ok(None),
            Some(pk) => match self.payment_key_mapper.get(&pk) {
                None => return Ok(None),
                Some(pk) => pk,
            },
        };

        let sk = match inner.staking_cred() {
            None => None,
            Some(sk) => self.staking_key_mapper.get(&sk),
        };

        Ok(Some((pk, sk)))
    }

    fn map_policy_id(&mut self, policy_id: u64) -> anyhow::Result<PolicyId> {
        let policy_id = self
            .policy_id_mapper
            .get_by_index(policy_id)
            .ok_or_else(|| anyhow!("can't find policy id: {}", policy_id))?;
        Ok(PolicyId::new(policy_id.to_string()))
    }

    fn map_policy_id_to_index(&mut self, policy_id: PolicyId) -> anyhow::Result<u64> {
        let policy_id = PolicyID::from_hex(policy_id.as_ref())
            .map_err(|err| anyhow!("can't decode policy id: {}, err: {}", policy_id, err))?;
        let policy_id = self
            .policy_id_mapper
            .get(&policy_id)
            .ok_or_else(|| anyhow!("policy id is not found: {}", policy_id))?;
        Ok(policy_id)
    }

    fn map_asset_name(&mut self, asset_name: u64) -> anyhow::Result<AssetName> {
        let asset_name = self
            .asset_name_mapper
            .get_by_index(asset_name)
            .ok_or_else(|| anyhow!("can't find asset name: {}", asset_name))?;
        Ok(AssetName::new(asset_name.clone()))
    }

    fn map_asset_name_to_index(&mut self, asset_name: AssetName) -> anyhow::Result<u64> {
        let asset_name = self
            .asset_name_mapper
            .get(&asset_name.to_string())
            .ok_or_else(|| anyhow!("asset name id is not found: {}", asset_name))?;

        Ok(asset_name)
    }

    fn map_token_id(&mut self, token: TokenId) -> anyhow::Result<(PolicyId, AssetName)> {
        match self.mapped_tokens.get(token.as_ref()).cloned() {
            None => Err(anyhow!("can't map token: {}", token.as_ref())),
            Some(result) => Ok(result),
        }
    }

    fn map_policy_id_and_asset(
        &mut self,
        policy: PolicyId,
        asset: AssetName,
    ) -> anyhow::Result<TokenId> {
        let fingerprint = dcspark_core::fingerprint(&policy, &asset)?;
        self.mapped_tokens
            .insert(fingerprint.to_string(), (policy, asset));
        Ok(fingerprint)
    }
}

#[cfg(test)]
mod tests {
    use crate::bench_utils::address_mapper::{CardanoDataMapper, StringAddressMapper};
    use dcspark_core::{Address, AssetName, PolicyId, TokenId};

    #[test]
    fn check_string_mapper() {
        let mut mapper = StringAddressMapper::default();
        assert_eq!(
            mapper.map_address(Some((0, Some(1)))).unwrap(),
            Address::new("0:1")
        );
        assert_eq!(
            mapper.map_address(Some((44, Some(1)))).unwrap(),
            Address::new("44:1")
        );
        assert_eq!(
            mapper.map_address(Some((44, None))).unwrap(),
            Address::new("44:null")
        );
        assert_eq!(mapper.map_address(None).unwrap(), Address::new("null"));

        assert_eq!(
            mapper.map_address_to_indices(Address::new("44:1")).unwrap(),
            Some((44, Some(1)))
        );
        assert_eq!(
            mapper
                .map_address_to_indices(Address::new("44:null"))
                .unwrap(),
            Some((44, None))
        );
        assert_eq!(
            mapper.map_address_to_indices(Address::new("null")).unwrap(),
            None
        );
        assert!(mapper.map_address_to_indices(Address::new("44:")).is_err());
        assert!(mapper.map_address_to_indices(Address::new("44")).is_err());
        assert!(mapper.map_address_to_indices(Address::new(":1")).is_err());
        assert!(mapper.map_address_to_indices(Address::new(":")).is_err());

        assert_eq!(
            mapper.map_token_id(TokenId::new("44:1")).unwrap(),
            (PolicyId::new("44"), AssetName::new("1"))
        );

        assert!(mapper.map_token_id(TokenId::new("44:")).is_err());
        assert!(mapper.map_token_id(TokenId::new(":5")).is_err());
        assert!(mapper.map_token_id(TokenId::new(":")).is_err());
        assert!(mapper.map_token_id(TokenId::new("44")).is_err());

        assert_eq!(
            mapper
                .map_policy_id_and_asset(PolicyId::new("44"), AssetName::new("1"))
                .unwrap(),
            TokenId::new("44:1")
        );
    }
}

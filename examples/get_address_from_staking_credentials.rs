use clap::Parser;

#[derive(Parser, Debug)]
#[clap(version)]
pub struct Cli {
    #[clap(long, value_parser)]
    payment_key: String,

    #[clap(long, value_parser)]
    staking_key: String,
}

fn main() {
    let Cli {
        payment_key,
        staking_key,
    } = Cli::parse();

    let payment_key = hex::decode(payment_key).unwrap();
    let staking_key = hex::decode(staking_key).unwrap();

    let payment_key =
        cardano_multiplatform_lib::address::StakeCredential::from_bytes(payment_key).unwrap();
    let staking_key =
        cardano_multiplatform_lib::address::StakeCredential::from_bytes(staking_key).unwrap();

    let addr = cardano_multiplatform_lib::address::BaseAddress::new(1, &payment_key, &staking_key);

    println!("{}", addr.to_address().to_bech32(None).unwrap());
}

// pk 8200581c9566a8f301fb8a046e44557bb38dfb9080a1213f17f200dcd3808169
// sk 8200581c49f14106ef746c2d3597381d1d5d1c65c91e933acd1baef3fc915f0b
// addr1qx2kd28nq8ac5prwg32hhvudlwggpgfp8utlyqxu6wqgz62f79qsdmm5dsknt9ecr5w468r9ey0fxwkdrwh08ly3tu9sy0f4qd

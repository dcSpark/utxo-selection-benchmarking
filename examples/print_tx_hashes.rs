use anyhow::anyhow;
use clap::Parser;
use entity::prelude::TransactionModel;

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::str::FromStr;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

#[derive(Parser, Debug)]
#[clap(version)]
pub struct Cli {
    /// path to config file
    #[clap(long, value_parser)]
    file_path: PathBuf,
}

#[tokio::main]
async fn main() {
    let result = _main().await;
    result.unwrap();
}

async fn _main() -> anyhow::Result<()> {
    let fmt_layer = tracing_subscriber::fmt::layer().with_test_writer();

    tracing_subscriber::registry().with(fmt_layer).init();

    let Cli { file_path } = Cli::parse();

    let unparsed_txs_file = if file_path.exists() && file_path.is_file() {
        File::open(file_path)?
    } else {
        return Err(anyhow!("can't open input file: {:?}", file_path));
    };

    let mut unparsed_addresses_file_lines = BufReader::new(unparsed_txs_file).lines();
    let count = u64::from_str(unparsed_addresses_file_lines.next().unwrap()?.as_str())?;
    let mut seen = 0;
    for line in unparsed_addresses_file_lines {
        let tx: TransactionModel = serde_json::from_str(line?.as_str())?;
        println!("hash: {}", hex::encode(tx.hash.clone()));
        seen += 1;
    }

    assert_eq!(seen, count);
    Ok(())
}

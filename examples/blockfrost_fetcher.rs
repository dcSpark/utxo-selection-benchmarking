use anyhow::{anyhow, Context};
use clap::Parser;
use dcspark_core::tx::TransactionId;
use dcspark_core::BlockNumber;
use reqwest::header::{HeaderMap, HeaderValue};
use reqwest::{header, Client, StatusCode};
use serde::Deserialize;
use std::fmt;
use std::fs::File;
use std::io::Write;

use std::path::PathBuf;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    endpoint: String,
    key: String,
    address: String,
    txs_output_path: PathBuf,
    retries: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct BlockfrostTransaction {
    pub tx_hash: TransactionId,
    pub tx_index: u64,
    pub block_time: u64,
    pub block_height: BlockNumber,
}

#[derive(Clone, Debug, Deserialize)]
pub struct BlockFrostError {
    pub status_code: usize,
    pub error: String,
    pub message: String,
}

#[derive(Parser, Debug)]
#[clap(version)]
pub struct Cli {
    /// path to config file
    #[clap(long, value_parser)]
    config_path: PathBuf,
}

#[tokio::main]
async fn main() {
    let result = _main().await;
    result.unwrap();
}

async fn _main() -> anyhow::Result<()> {
    // Start logging setup block
    let fmt_layer = tracing_subscriber::fmt::layer().with_test_writer();

    tracing_subscriber::registry().with(fmt_layer).init();

    let Cli { config_path } = Cli::parse();

    tracing::info!("Config file {:?}", config_path);
    let file = File::open(&config_path).with_context(|| {
        format!(
            "Cannot read config file {path}",
            path = config_path.display()
        )
    })?;
    let config: Config = serde_yaml::from_reader(file).with_context(|| {
        format!(
            "Cannot read config file {path}",
            path = config_path.display()
        )
    })?;

    let mut headers = HeaderMap::new();
    headers.append(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/cbor"),
    );
    headers.append(
        "project_id",
        HeaderValue::from_str(&config.key)
            .context("The project_id (authentication key) is not in a valid format")?,
    );

    let client = reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .context("Failed to build HTTP Client")?;

    let mut output = File::create(config.txs_output_path.clone())?;
    let mut last_transactions_count = 1;
    let mut page = 1;
    while last_transactions_count > 0 {
        let mut retires = config.retries;
        let mut transactions = vec![];
        while retires > 0 {
            let txs = get_txs(&client, &config, page).await;
            if let Ok(txs) = txs {
                transactions = txs;
                break;
            } else {
                retires -= 1;
                if retires == 0 {
                    return Err(anyhow!("retires limit reached at page {}", page));
                }
            }
        }

        last_transactions_count = transactions.len();

        for tx in transactions {
            output.write_all(format!("{} {}\n", tx.block_height, tx.tx_hash).as_bytes())?;
        }
        page += 1;
    }

    Ok(())
}

fn url(endpoint: &String, api: impl fmt::Display) -> String {
    format!("{endpoint}/{api}")
}

async fn get_txs(
    client: &Client,
    config: &Config,
    page: u64,
) -> anyhow::Result<Vec<BlockfrostTransaction>> {
    let req = client
        .get(url(
            &config.endpoint,
            format!(
                "api/v0/addresses/{}/transactions?page={}",
                config.address, page
            ),
        ))
        .send()
        .await
        .context("Failed to get transactions from blockfrost endpoint")?;

    match req.status() {
        StatusCode::OK => {
            let txs: Vec<BlockfrostTransaction> = req
                .json()
                .await
                .context("Expect the endpoint to return transaction data")?;

            Ok(txs)
        }
        code => Err(anyhow!("error: {:?}", code)),
    }
}

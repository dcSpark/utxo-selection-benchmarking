use std::path::PathBuf;

use anyhow::{anyhow, Context};
use cardano_multiplatform_lib::address::StakeCredential;

use cardano_multiplatform_lib::PolicyID;
use clap::Parser;

use entity::sea_orm::Database;
use entity::sea_orm::QueryFilter;
use entity::{
    prelude::*,
    sea_orm::{prelude::*, ColumnTrait, Condition, EntityTrait, QueryOrder, QuerySelect},
};
use serde::Deserialize;
use std::cmp::min;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::Write;
use tracing_subscriber::prelude::*;
use utxo_selection_benchmark::generation_utils::{
    carp_tx_to_events, clean_events, dump_unparsed_transactions_to_file,
};
use utxo_selection_benchmark::mapper::DataMapper;
use utxo_selection_benchmark::tx_event::TxOutput;
use utxo_selection_benchmark::utils::dump_hashset_to_file;

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
pub enum DbConfig {
    Postgres {
        host: String,
        port: u64,
        user: String,
        password: String,
        db: String,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    db: DbConfig,
    unparsed_transactions: PathBuf,
    payment_creds_mapping: PathBuf,
    staking_creds_mapping: PathBuf,
    policy_mapping: PathBuf,
    asset_name_mapping: PathBuf,
    banned_addresses: PathBuf,
    events_output_path: PathBuf,
    cleaned_events_output_path: PathBuf,
    tx_per_page: i64,
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

    let sqlx_filter = tracing_subscriber::filter::Targets::new()
        // sqlx logs every SQL query and how long it took which is very noisy
        .with_target("sqlx", tracing::Level::WARN)
        .with_default(tracing_subscriber::fmt::Subscriber::DEFAULT_MAX_LEVEL);

    tracing_subscriber::registry()
        .with(fmt_layer)
        .with(sqlx_filter)
        .init();

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
    let (user, password, host, port, db) = match config.db {
        DbConfig::Postgres {
            host,
            port,
            user,
            password,
            db,
        } => (user, password, host, port, db),
    };

    let url = format!("postgresql://{user}:{password}@{host}:{port}/{db}");
    tracing::info!("Connection url {:?}", url);
    let conn = Database::connect(&url).await?;
    tracing::info!("Connection success");

    /////////
    let shelley_first_blocks = Block::find()
        .filter(BlockColumn::Epoch.eq(208))
        .order_by_asc(BlockColumn::Id)
        .limit(256)
        .all(&conn)
        .await?;
    tracing::info!(
        "Shelley first blocks {:?}",
        shelley_first_blocks
            .iter()
            .map(|block| (block.id, block.height, block.epoch))
            .collect::<Vec<(i32, i32, i32)>>()
    );
    let shelley_first_blocks: Vec<i32> =
        shelley_first_blocks.iter().map(|block| block.id).collect();

    let mut condition = Condition::any();
    for block in shelley_first_blocks {
        condition = condition.add(TransactionColumn::BlockId.eq(block));
    }
    let shelley_first_tx: Vec<i64> = Transaction::find()
        .filter(condition)
        .order_by_asc(TransactionColumn::Id)
        .limit(1)
        .all(&conn)
        .await?
        .iter()
        .map(|tx| tx.id)
        .collect();
    let shelley_first_tx = shelley_first_tx
        .first()
        .cloned()
        .ok_or_else(|| anyhow!("Can't find first tx"))?;
    tracing::info!("Shelley first tx, {:?}", shelley_first_tx);

    //////////////

    let transactions = Transaction::find()
        .filter(TransactionColumn::Id.gte(shelley_first_tx))
        .order_by_asc(TransactionColumn::Id)
        .paginate(&conn, config.tx_per_page as usize);
    let total_transactions = transactions.num_items().await?;
    let total_pages = transactions.num_pages().await?;
    tracing::info!("Total transactions: {:?}", total_transactions);
    tracing::info!("Total pages: {:?}", total_pages);

    let mut out_file = if config.events_output_path.exists() && config.events_output_path.is_file()
    {
        tracing::info!(
            "file {:?} already exists, adding lines to the end",
            config.events_output_path
        );
        File::open(config.events_output_path.clone())
    } else {
        File::create(config.events_output_path.clone())
    }?;

    let mut current_start = shelley_first_tx;
    let mut current_end = shelley_first_tx + config.tx_per_page;
    let max_end = shelley_first_tx + total_transactions as i64;

    let mut current_query = Transaction::find()
        .filter(
            Condition::all()
                .add(TransactionColumn::Id.gte(current_start))
                .add(TransactionColumn::Id.lt(current_end)),
        )
        .order_by_asc(TransactionColumn::Id)
        .all(&conn)
        .await?;

    let mut previous_outputs = HashMap::<String, HashMap<u64, TxOutput>>::new();

    let mut stake_address_to_num = DataMapper::<StakeCredential>::new();
    let mut payment_address_to_num = DataMapper::<StakeCredential>::new();
    let mut policy_id_to_num = DataMapper::<PolicyID>::new();
    let mut asset_name_to_num = DataMapper::<String>::new();
    let mut banned_addresses = HashSet::<(u64, Option<u64>)>::new();

    let mut unparsed_transactions = Vec::<TransactionModel>::new();

    while !current_query.is_empty() {
        let tx_count = current_query.len();
        tracing::info!(
            "fetched txs from {:?} to {:?}, total: {:?}, max: {:?}",
            current_start,
            current_end,
            tx_count,
            max_end
        );
        for tx in current_query {
            let tx_event = carp_tx_to_events(
                &tx,
                &mut previous_outputs,
                &mut stake_address_to_num,
                &mut payment_address_to_num,
                &mut policy_id_to_num,
                &mut asset_name_to_num,
                &mut banned_addresses,
                &mut unparsed_transactions,
            )?;
            if let Some(tx_event) = tx_event {
                out_file
                    .write_all(format!("{}\n", serde_json::to_string(&tx_event)?).as_bytes())?;
            }
        }

        current_start = current_end;
        current_end += config.tx_per_page;
        current_end = min(current_end, max_end);
        current_query = Transaction::find()
            .filter(
                Condition::all()
                    .add(TransactionColumn::Id.gte(current_start))
                    .add(TransactionColumn::Id.lt(current_end)),
            )
            .order_by_asc(TransactionColumn::Id)
            .all(&conn)
            .await?;
    }

    drop(out_file);

    tracing::info!("Parsing finished, dumping files");
    tracing::info!(
        "Total unparsed transactions: {:?}",
        unparsed_transactions.len()
    );

    dump_unparsed_transactions_to_file(config.unparsed_transactions, unparsed_transactions)?;

    payment_address_to_num.dump_to_file(config.payment_creds_mapping)?;
    stake_address_to_num.dump_to_file(config.staking_creds_mapping)?;
    policy_id_to_num.dump_to_file(config.policy_mapping)?;
    asset_name_to_num.dump_to_file(config.asset_name_mapping)?;
    dump_hashset_to_file(&banned_addresses, config.banned_addresses)?;

    tracing::info!("Dumping finished, cleaning events");

    clean_events(
        config.events_output_path,
        config.cleaned_events_output_path,
        &banned_addresses,
    )?;

    tracing::info!("Cleaning finished");

    Ok(())
}

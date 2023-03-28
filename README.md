# UTxO selection benchmark

Along with the UTxO selection library we provide utxo selection algorithms benchmarking library.
This library includes: 
* carp events fetcher & filter -- the tool that takes events from carp database, so the algorithms can be benchmarked using them
* benchmarking tool itself

This library can be used to compare the algorithms: how they behave, how they affect the fees, what will be the final utxo sets and so on.

## Core principles
Core principle of the library is modularity. To conduct an experiment you will need to define:
* Main input selection algorithm
* Change balancing algorithm (not mandatory)
* Transaction fee estimator
* Data mapper (how to convert ids of credentials/asset names to sth real if it's needed)
* Staking keys ids in which you're interested
* Paths of benchmarking results

## Single address benchmarking
**Requirements**:

To fetch events for single address you will need:
* blockfrost api key 
* carp deployment

**Step by step guide**:
1. Patch [configs/blockfrost_fetcher](configs/blockfrost_fetcher.yml) by providing your api key and address:
```yaml
endpoint: https://cardano-mainnet.blockfrost.io
key: <enter your key>
address: <your cardano address>
txs_output_path: address_transactions.txt
retries: 5
```
2. Create a folder for events: `mkdir my_events && cd my_events`
3. Run `cargo run --release --example blockfrost_fetcher -- --config-path ../configs/blockfrost_fetcher.yml`
4. Wait until `my_events/address_transactions.txt` file is populated with the transaction ids related to your address
5. Patch [configs/carp_single_address_fetcher](configs/carp_single_address_fetcher.yml) by providing carp credentials:
```yaml
db:
  type: postgres
  host: localhost
  port: 5432
  user: carp
  password: carpdb
  db: carp

payment_creds_mapping: payment_credentials.mapping
staking_creds_mapping: staking_credentials.mapping
policy_mapping: policy_id.mapping
asset_name_mapping: asset_name.mapping

unparsed_transactions: unparsed_transactions.txt
banned_addresses: banned_addresses.txt

events_output_path: raw_events.ev
input_transactions_path: address_transactions.txt
```
6. Run `cargo run --release --example carp_single_address_fetcher -- --config-path ../configs/carp_single_address_fetcher.yml`
7. The script will show sth like:
```text
2023-03-28T01:57:51.760391Z  INFO carp_single_address_fetcher: Connection success
2023-03-28T02:00:18.942242Z  INFO carp_single_address_fetcher: Parsing finished, dumping files
2023-03-28T02:00:18.942283Z  INFO carp_single_address_fetcher: Total unparsed transactions: 0
2023-03-28T02:00:19.279139Z  INFO carp_single_address_fetcher: Dumping finished
```
8. If there're unparsed transactions -- please remove them from the list and submit an issue to us (if the transaction is not related to byron address / buggy addresses)
   1. Besides, the script would generate files with mappings and events in the same `my_events` folder
9. Now create a folder `bench_result` and patch [configs/run_benchmark](configs/run_benchmark.yml) to choose the algorithm you prefer:
```yaml
paths:
  events_path: "raw_events.ev"
  output_insolvent: "bench_result/insolvent_addresses.txt"
  output_discarded: "bench_result/discarded_addresses.txt"
  output_balance: "bench_result/balances.txt"
  output_balance_short: "bench_result/short_stats.txt"
  utxos_path: "bench_result/final_utxos.txt"

algo:
  type: largest_first
#  type: thermostat

change_balance_algo:
  type: single_change

# thermostat estimator is optimized for native scripts
#fee_estimator:
#  type: thermostat
#  network: mainnet
#  plan_path: "events/milkomeda_events/multisig.script"

fee_estimator:
  type: cml_estimator
  magic: "mainnet.cardano-evm.c1"
  plan_path: "events/milkomeda_events/multisig.script"

# the most advanced mapper which works with cml estimator well
mapper:
  type: cml_mapper
  payment_key_path: "payment_credentials.mapping"
  staking_key_path: "staking_credentials.mapping"
  policy_id_path: "policy_id.mapping"
  asset_name_path: "asset_name.mapping"
  network: 1
  default_address: "addr1qx2kd28nq8ac5prwg32hhvudlwggpgfp8utlyqxu6wqgz62f79qsdmm5dsknt9ecr5w468r9ey0fxwkdrwh08ly3tu9sy0f4qd"

# staking key id which you want to monitor
keys_of_interest: [9999999]

# specify if you want to use separate change algo
allow_balance_change: true
```
10. Make sure to specify the id of your staking key in the aforementioned config. You can easily identify it in `raw_events.ev`. `get_address_from_staking_credentials` script can help you as well.
11. Run `cargo run --release --example run_benchmark -- --config-path ../configs/run_benchmark.yml`
    1. If the execution went well you will see sth like:
```text
2023-03-28T04:08:25.738422Z  INFO utxo_selection_benchmark::bench: Total converged addresses: 1
2023-03-28T04:08:25.738435Z  INFO utxo_selection_benchmark::bench: Total insolvent addresses: 1532
2023-03-28T04:08:25.738437Z  INFO utxo_selection_benchmark::bench: Total banned addresses: 233
```
12. In `my_events/bench_result` you will see multiple files: 
    1. balances.txt -- contains balances of converged addresses     
    2. discarded_addresses.txt -- contains the staking key ids which were excluded from consideration due to a) interest b) participation in insolvent txs 
    3. final_utxos.txt -- contains final utxo sets of the addresses of interest. In the end of file you will find short aggregation like:
       1. `total greater than 10: 11831`
       2. `total less than 10: 6354`
    4. insolvent_addresses.txt -- contains the staking key ids which were insolvent during experiment
    5. short_stats.txt -- contains short stats

## Multi address benchmarking

The library provides an option to conduct an experiment on the whole blockchain (since shelley era).
Step-by-step instruction:
1. Patch [configs/carp_fetcher.yml](configs/carp_fetcher.yml):
```yaml
db:
  type: postgres
  host: localhost
  port: 5432
  user: carp
  password: carpdb
  db: carp

tx_per_page: 4096

payment_creds_mapping: payment_credentials.mapping
staking_creds_mapping: staking_credentials.mapping
policy_mapping: policy_id.mapping
asset_name_mapping: asset_name.mapping

banned_addresses: banned_addresses.txt
unparsed_transactions: unparsed_transactions.txt

events_output_path: raw_events.ev
cleaned_events_output_path: cleaned_events.ev
```
2. Create folder for events: `mkdir my_events && cd my_events`
3. Run `cargo run --release --example carp_fetcher -- --config-path ../configs/carp_fetcher.yml`
4. You will get the events in `cleaned_events.ev` file
5. In case you face unsupported transactions - collect the addresses that participate in them into a file called `unparsed_transaction_addresses.txt`
   1. Patch [configs/finish_events_parsing.yml](configs/finish_events_parsing.yml) (same format as other configs)
   2. Run `cargo run --release --example finish_events_parsing -- --config-path ../configs/finish_events_parsing.yml`
   3. These addresses will be excluded from mappings, events list and so on
6. Run the benchmark like in section above
   1. Don't forget to empty `keys_of_interest` field in the config. Otherwise, only addresses from that list will participate in the benchmark.

## Limitations:

* Byron addresses / byron transactions are not supported
* If the transaction has inputs from > 1 staking keys it is considered invalid and these keys won't participate in the experiment
  * This was done to identify for sure which address should be used for changes
* Not so many sources of events are supported right now
* Addresses without staking key are not supported, but it can be fixed easily by following replacement:
  * `sed -i '' "s/\[0,null\]/\[0,9999999\]/gi" my_events/raw_events.ev`
  * Here 0 is id of your payment key, 9999999 will be your fake staking key
  * If you're using `cml_mapper` -- add string like that into `staking_credentials.mapping` file and increment the number on the first line of this file:
    * `"8200581c49f14106ef746c2d3597381d1d5d1c65c91e933acd1baef3fc915f0b":9999999`

We're always open for contributions, if you want to participate in the project in any way please let us know

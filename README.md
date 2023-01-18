# UTxO selection benchmark

Along with utxo selection library we provide utxo selection algorithms benchmarking library.
This library includes: 
* carp events fetcher & filter -- the tool that takes events from carp database, so the algorithms can be benchmarked using them
* benchmarking tool itself

This library can be used to compare the algorithms: how they behave, how they affect the fees, what will be the final utxo sets and so on.

Currently, the library supports only events with inputs balance = outputs balance + fee.

To run the benchmark on all fetched events use `run_benchmark` example, it will process the events and give short and detailed stats. 
Take a look at the example benchmark config at [bench](configs/run_benchmark.yml). Short stats will be sth like:
```text
better than actual: 122922
not worse as actual: 620515
worse than actual: 544922
can't compare: 662537
not found actual: 0
not found token actual: 0
```

The detailed stats include: 
* final balances `bench_result/balances.txt`
* final utxos `bench_result/final_utxos.txt`
* addresses that couldn't converge: `bench_result/insolvent_addresses.txt`
* addresses that were discarded: `bench_result/discarded_addresses.txt`
  * an address can be discarded if there is more than 1 staking keys participating in input selection (in history). That's needed to be able to identify correctly the change addresses.

The benchmark can be run for any amount of events, for single address as well.
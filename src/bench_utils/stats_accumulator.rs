use std::collections::HashMap;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

#[derive(Default)]
pub struct StatsAccumulator<T: ToString> {
    points: HashMap<u64, Vec<(u64, T)>>,
}

impl<T: ToString> StatsAccumulator<T> {
    pub fn add_stats(&mut self, stake_key: u64, point: u64, data: T) {
        let point_of_stake = self.points.entry(stake_key).or_insert(vec![]);
        point_of_stake.push((point, data));
    }

    pub fn dump_stats(&self, path: PathBuf, format: String) -> anyhow::Result<()> {
        let mut stats = File::create(path)?;
        stats.write_all(format!("stake_key,index,{}\n", format).as_bytes())?;
        for (stake_key, points) in self.points.iter() {
            for (index, data) in points {
                stats.write_all(
                    format!("{},{},{}\n", stake_key, index, data.to_string()).as_bytes(),
                )?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Default, Clone)]
pub struct BalanceStats {
    pub ada_computed: i64,
    pub ada_actual: i64,
    pub fee_computed: i64,
    pub fee_actual: i64,
}

impl ToString for BalanceStats {
    fn to_string(&self) -> String {
        format!(
            "{},{},{},{}",
            self.ada_computed, self.ada_actual, self.fee_computed, self.fee_actual
        )
    }
}

#[cfg(test)]
mod tests {
    use crate::bench_utils::stats_accumulator::BalanceStats;

    #[test]
    fn check_serialize() {
        assert_eq!(BalanceStats::default().to_string(), String::from("0,0,0,0"));
    }
}

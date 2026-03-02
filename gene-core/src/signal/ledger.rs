use crate::signal::types::SignalSnapshot;
use anyhow::Result;
use sled::Db;
use std::path::Path;

/// Append-only persistent log of signal snapshots.
/// Every tick writes a snapshot here. This is the raw memory of the system.
pub struct SignalLedger {
    db: Db,
    /// Track total entries for quota enforcement
    entry_count: u64,
    /// Max entries before compaction (oldest are dropped)
    quota: u64,
}

impl SignalLedger {
    pub fn open(path: &Path, quota: u64) -> Result<Self> {
        let db = sled::open(path)?;
        let entry_count = db.len() as u64;
        Ok(Self { db, entry_count, quota })
    }

    /// Append a snapshot. Returns the tick key used.
    pub fn append(&mut self, snapshot: &SignalSnapshot) -> Result<()> {
        let key = snapshot.tick.to_be_bytes();
        let value = bincode::serialize(snapshot)?;
        self.db.insert(key, value)?;
        self.entry_count += 1;

        if self.entry_count > self.quota {
            self.compact(self.quota / 4)?;
        }

        Ok(())
    }

    /// Read snapshots in a tick range [from, to].
    pub fn range(&self, from: u64, to: u64) -> Result<Vec<SignalSnapshot>> {
        let from_key = from.to_be_bytes();
        let to_key = to.to_be_bytes();
        let mut out = Vec::new();
        for item in self.db.range(from_key..=to_key) {
            let (_, v) = item?;
            let snap: SignalSnapshot = bincode::deserialize(&v)?;
            out.push(snap);
        }
        Ok(out)
    }

    /// Read the last N snapshots.
    pub fn tail(&self, n: usize) -> Result<Vec<SignalSnapshot>> {
        let mut out: Vec<SignalSnapshot> = Vec::with_capacity(n);
        for item in self.db.iter().rev().take(n) {
            let (_, v) = item?;
            let snap: SignalSnapshot = bincode::deserialize(&v)?;
            out.push(snap);
        }
        out.reverse();
        Ok(out)
    }

    pub fn len(&self) -> u64 {
        self.entry_count
    }

    /// Drop oldest `n` entries to stay under quota.
    fn compact(&mut self, n: u64) -> Result<()> {
        let mut removed = 0u64;
        for item in self.db.iter() {
            if removed >= n {
                break;
            }
            let (k, _) = item?;
            self.db.remove(k)?;
            removed += 1;
        }
        self.entry_count = self.entry_count.saturating_sub(removed);
        tracing::info!("ledger compacted: removed {} entries", removed);
        Ok(())
    }

    pub fn flush(&self) -> Result<()> {
        self.db.flush()?;
        Ok(())
    }
}

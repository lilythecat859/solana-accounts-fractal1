//! Fractal-Sharded: 4 k shards for zero-contention reads
use dashmap::DashMap;
use solana_sdk::{account::Account, pubkey::Pubkey};
use std::sync::Arc;

const SHARD_BITS: usize = 12; // 4096 shards
const SHARD_MASK: usize = (1 << SHARD_BITS) - 1;

pub struct ShardedIndex {
    shards: Vec<DashMap<Pubkey, Arc<Account>>>,
}

impl Default for ShardedIndex {
    fn default() -> Self {
        let shards = (0..(1 << SHARD_BITS))
            .map(|_| DashMap::with_capacity(1024))
            .collect();
        Self { shards }
    }
}

impl ShardedIndex {
    pub fn insert(&self, key: Pubkey, acc: Account) {
        let shard = &self.shards[(key.as_ref()[0] as usize) & SHARD_MASK];
        shard.insert(key, Arc::new(acc));
    }

    pub fn get(&self, key: &Pubkey) -> Option<Arc<Account>> {
        let shard = &self.shards[(key.as_ref()[0] as usize) & SHARD_MASK];
        shard.get(key).map(|e| e.clone())
    }

    pub fn get_program_accounts(&self, program: &Pubkey) -> Vec<(Pubkey, Arc<Account>)> {
        let shard_idx = (program.as_ref()[0] as usize) & SHARD_MASK;
        let shard = &self.shards[shard_idx];
        shard
            .iter()
            .filter(|entry| entry.value().owner == *program)
            .map(|entry| (*entry.key(), entry.value().clone()))
            .collect()
    }

    pub fn remove(&self, key: &Pubkey) -> Option<Arc<Account>> {
        let shard = &self.shards[(key.as_ref()[0] as usize) & SHARD_MASK];
        shard.remove(key).map(|(_k, v)| v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shard_works() {
        let idx = ShardedIndex::default();
        let pk = Pubkey::new_unique();
        idx.insert(pk, Account::default());
        assert!(idx.get(&pk).is_some());
    }
}

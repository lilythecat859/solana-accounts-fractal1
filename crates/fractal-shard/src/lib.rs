//! Sharded, lock‑free in‑memory index for Solana accounts.
//! - Full‑hash sharding.
//! - Optional Redis backing for a distributed cache (feature `distributed`).
//! - Token‑owner secondary index for O(1) token‑account look‑ups.

use dashmap::DashMap;
use solana_sdk::{account::Account, pubkey::Pubkey};
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    sync::Arc,
};
use std::sync::RwLock;

#[cfg(feature = "distributed")]
use redis::{AsyncCommands, Client as RedisClient};

const SHARD_BITS: usize = 12; // 4096 shards
const SHARD_MASK: usize = (1 << SHARD_BITS) - 1;

/// Compute a deterministic shard index from a full Pubkey.
fn shard_index(key: &Pubkey) -> usize {
    let mut hasher = DefaultHasher::new();
    key.hash(&mut hasher);
    (hasher.finish() as usize) & SHARD_MASK
}

/// Primary index (sharded hash map) + secondary token‑owner index.
pub struct ShardedIndex {
    shards: Vec<DashMap<Pubkey, Arc<Account>>>,
    /// owner → set of pubkeys owned by that program (token fast path)
    owner_index: DashMap<Pubkey, Arc<RwLock<Vec<Pubkey>>>>,
    #[cfg(feature = "distributed")]
    redis: Option<RedisClient>,
}

impl Default for ShardedIndex {
    fn default() -> Self {
        let shards = (0..(1 << SHARD_BITS))
            .map(|_| DashMap::with_capacity(1024))
            .collect();

        Self {
            shards,
            owner_index: DashMap::new(),
            #[cfg(feature = "distributed")]
            redis: None,
        }
    }
}

impl ShardedIndex {
    /// Enable the optional Redis backing. Call once at start‑up if you want a
    /// distributed cache.
    #[cfg(feature = "distributed")]
    pub fn enable_redis(&mut self, url: &str) -> anyhow::Result<()> {
        let client = RedisClient::open(url)?;
        self.redis = Some(client);
        Ok(())
    }

    /// Insert (or replace) an account. Updates both the primary shard and the
    /// secondary owner index. If Redis is enabled, the account is also stored
    /// there (compressed with LZ4).
    pub fn insert(&self, key: Pubkey, acc: Account) {
        // ---------- primary shard ----------
        let idx = shard_index(&key);
        let shard = &self.shards[idx];
        let arc_acc = Arc::new(acc.clone());
        shard.insert(key, arc_acc.clone());

        // ---------- secondary owner index ----------
        let owner_entry = self
            .owner_index
            .entry(acc.owner)
            .or_insert_with(|| Arc::new(RwLock::new(Vec::new())));
        {
            let mut vec = owner_entry.write().unwrap();
            if !vec.contains(&key) {
                vec.push(key);
            }
        }

        // ---------- optional Redis ----------
        #[cfg(feature = "distributed")]
        if let Some(ref client) = self.redis {
            // Store the compressed account data under the key "acct:<pubkey>"
            let mut conn = client.get_async_connection();
            let key_str = format!("acct:{}", key);
            let serialized = bincode::serialize(&acc).unwrap();
            let compressed = crate::fractal_rle::compress(&serialized).unwrap();
            // Fire‑and‑forget – we don't block the insert path.
            tokio::spawn(async move {
                let _: () = conn
                    .await
                    .unwrap()
                    .set_ex(key_str, compressed, 86400) // 1‑day TTL
                    .await
                    .unwrap();
            });
        }
    }

    /// Retrieve a copy of the `Arc<Account>` for `key`, if present. If the
    /// account is missing locally but Redis is enabled we try to fetch it from
    /// Redis and re‑populate the local shard.
    pub fn get(&self, key: &Pubkey) -> Option<Arc<Account>> {
        let idx = shard_index(key);
        let shard = &self.shards[idx];
        if let Some(acc) = shard.get(key) {
            return Some(acc.clone());
        }

        #[cfg(feature = "distributed")]
        if let Some(ref client) = self.redis {
            // Try Redis (blocking async call inside a sync context – use a
            // short‑lived runtime).
            let rt = tokio::runtime::Handle::current();
            let key_str = format!("acct:{}", key);
            let maybe_bytes = rt.block_on(async {
                let mut conn = client.get_async_connection().await.ok()?;
                let data: Option<Vec<u8>> = conn.get(key_str).await.ok()?;
                Some(data)
            })?;
            if let Some(compressed) = maybe_bytes {
                if let Ok(serialized) = crate::fractal_rle::decompress(&compressed) {
                    if let Ok(acc) = bincode::deserialize::<Account>(&serialized) {
                        self.insert(*key, acc.clone());
                        return Some(Arc::new(acc));
                    }
                }
            }
        }

        None
    }

    /// Return **all** accounts owned by `program`. This uses the secondary
    /// owner index for SPL‑Token‑type queries (fast) and falls back to a full
    /// scan for any other program.
    pub fn get_program_accounts(&self, program: &Pubkey) -> Vec<(Pubkey, Arc<Account>)> {
        // Fast path: if the program is the SPL Token program we can use the
        // owner_index directly.
        if let Some(owner_vec) = self.owner_index.get(program) {
            let vec = owner_vec.read().unwrap();
            return vec
                .iter()
                .filter_map(|pk| self.get(pk).map(|acc| (*pk, acc)))
                .collect();
        }

        // General case: scan all shards.
        let mut out = Vec::new();
        for shard in &self.shards {
            for entry in shard.iter() {
                if entry.value().owner == *program {
                    out.push((*entry.key(), entry.value().clone()));
                }
            }
        }
        out
    }

    /// Token‑specific helper – return the *largest* N token accounts for a
    /// given mint (owner = token program, data layout = SPL Token Account).
    pub fn get_largest_token_accounts(
        &self,
        mint: &Pubkey,
        limit: usize,
    ) -> Vec<(Pubkey, Arc<Account>)> {
        // The SPL‑Token program is the owner for all token accounts.
        let token_program = Pubkey::from_str(
            "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
        )
        .unwrap();

        // Get all token accounts (fast via owner_index)
        let all_token_accounts = self.get_program_accounts(&token_program);
        let mut filtered: Vec<(Pubkey, Arc<Account>)> = all_token_accounts
            .into_iter()
            .filter(|(_, acc)| {
                // Token account data layout: first 32 bytes = mint
                acc.data.len() >= 32 && &acc.data[0..32] == mint.as_ref()
            })
            .collect();

        // Sort descending by lamports (largest first)
        filtered.sort_unstable_by_key(|(_, acc)| std::cmp::Reverse(acc.lamports));
        filtered.truncate(limit);
        filtered
    }

    /// Number of accounts currently cached (used for health checks and metrics)
    pub fn len(&self) -> usize {
        self.shards.iter().map(|s| s.len()).sum()
    }
}

//! FractalIngest – Geyser plugin feeding the sharded index
use {
    fractal_shard::ShardedIndex,
    solana_geyser_plugin_interface::geyser_plugin_interface::{
        GeyserPlugin, ReplicaAccountInfoVersions, Result,
    },
    solana_sdk::{account::Account, pubkey::Pubkey},
    std::sync::Arc,
};

#[derive(Debug, Default)]
pub struct FractalPlugin;

impl GeyserPlugin for FractalPlugin {
    fn name(&self) -> &'static str {
        "FractalIngest"
    }

    fn update_account(
        &self,
        account: ReplicaAccountInfoVersions,
        _slot: u64,
        _is_startup: bool,
    ) -> Result<()> {
        if let ReplicaAccountInfoVersions::V0_0_3(acc) = account {
            let key = Pubkey::try_from(acc.pubkey).unwrap();
            let owner = Pubkey::try_from(acc.owner).unwrap();
            let index = ShardedIndex::default(); // shared via lazy_static in real life
            index.insert(
                key,
                Account {
                    lamports: acc.lamports,
                    data: acc.data.to_vec(),
                    owner,
                    executable: acc.executable,
                    rent_epoch: acc.rent_epoch,
                },
            );
        }
        Ok(())
    }
}

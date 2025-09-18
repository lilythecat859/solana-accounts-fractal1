//! Geyser plugin that feeds account updates into a shared `ShardedIndex`.
//! Errors are reported back to the validator instead of panicking.

use {
    fractal_shard::ShardedIndex,
    solana_geyser_plugin_interface::geyser_plugin_interface::{
        GeyserPlugin, ReplicaAccountInfoVersions, Result, SlotStatus, Error as GeyserError,
    },
    solana_sdk::{account::Account, pubkey::Pubkey},
    std::sync::Arc,
    tokio::runtime::Runtime,
};

pub struct FractalPlugin {
    index: Arc<ShardedIndex>,
    runtime: Runtime,
}

impl Default for FractalPlugin {
    fn default() -> Self {
        Self {
            index: Arc::new(ShardedIndex::default()),
            runtime: tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio runtime"),
        }
    }
}

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
        // Accept only the version we know; return a clear error for any future version.
        let acc = match account {
            ReplicaAccountInfoVersions::V0_0_3(a) => a,
            _ => {
                return Err(GeyserError::InvalidArgument(
                    "Unsupported ReplicaAccountInfo version".into(),
                ))
            }
        };

        // Convert the raw byte slices into `Pubkey`s, returning a proper Geyser error on failure.
        let key = Pubkey::try_from(acc.pubkey)
            .map_err(|e| GeyserError::InvalidArgument(format!("bad pubkey: {e}")))?;
        let owner = Pubkey::try_from(acc.owner)
            .map_err(|e| GeyserError::InvalidArgument(format!("bad owner: {e}")))?;

        // Store the account. `data` is cloned because the slice is only valid for the duration
        // of this callback.
        self.index.insert(
            key,
            Account {
                lamports: acc.lamports,
                data: acc.data.to_vec(),
                owner,
                executable: acc.executable,
                rent_epoch: acc.rent_epoch,
            },
        );

        Ok(())
    }

    fn account_data_notifications_enabled(&self) -> bool {
        true
    }
}

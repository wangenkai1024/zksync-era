use std::time::Duration;

use anyhow::Context;
use multivm::{
    interface::{L1BatchEnv, L2BlockEnv, SystemEnv, TxExecutionMode},
    vm_latest::constants::BLOCK_GAS_LIMIT,
};
use zksync_contracts::BaseSystemContracts;
use zksync_dal::StorageProcessor;
use zksync_types::{
    fee_model::BatchFeeInput, Address, L1BatchNumber, L2ChainId, MiniblockNumber,
    ProtocolVersionId, H256, U256, ZKPORTER_IS_AVAILABLE,
};
use zksync_utils::u256_to_h256;

use super::PendingBatchData;
use crate::state_keeper::extractors;

/// Returns the parameters required to initialize the VM for the next L1 batch.
#[allow(clippy::too_many_arguments)]
pub(crate) fn l1_batch_params(
    current_l1_batch_number: L1BatchNumber,
    fee_account: Address,
    l1_batch_timestamp: u64,
    previous_batch_hash: U256,
    fee_input: BatchFeeInput,
    first_miniblock_number: MiniblockNumber,
    prev_miniblock_hash: H256,
    base_system_contracts: BaseSystemContracts,
    validation_computational_gas_limit: u32,
    protocol_version: ProtocolVersionId,
    virtual_blocks: u32,
    chain_id: L2ChainId,
) -> (SystemEnv, L1BatchEnv) {
    (
        SystemEnv {
            zk_porter_available: ZKPORTER_IS_AVAILABLE,
            version: protocol_version,
            base_system_smart_contracts: base_system_contracts,
            gas_limit: BLOCK_GAS_LIMIT,
            execution_mode: TxExecutionMode::VerifyExecute,
            default_validation_computational_gas_limit: validation_computational_gas_limit,
            chain_id,
        },
        L1BatchEnv {
            previous_batch_hash: Some(u256_to_h256(previous_batch_hash)),
            number: current_l1_batch_number,
            timestamp: l1_batch_timestamp,
            fee_input,
            fee_account,
            enforced_base_fee: None,
            first_l2_block: L2BlockEnv {
                number: first_miniblock_number.0,
                timestamp: l1_batch_timestamp,
                prev_block_hash: prev_miniblock_hash,
                max_virtual_blocks_to_create: virtual_blocks,
            },
        },
    )
}

/// Returns the amount of iterations `delay_interval` fits into `max_wait`, rounding up.
pub(crate) fn poll_iters(delay_interval: Duration, max_wait: Duration) -> usize {
    let max_wait_millis = max_wait.as_millis() as u64;
    let delay_interval_millis = delay_interval.as_millis() as u64;
    assert!(delay_interval_millis > 0, "delay interval must be positive");

    ((max_wait_millis + delay_interval_millis - 1) / delay_interval_millis).max(1) as usize
}

pub(crate) async fn load_l1_batch_params(
    storage: &mut StorageProcessor<'_>,
    current_l1_batch_number: L1BatchNumber,
    fee_account: Address,
    validation_computational_gas_limit: u32,
    chain_id: L2ChainId,
) -> Option<(SystemEnv, L1BatchEnv)> {
    // If miniblock doesn't exist (for instance if it's pending), it means that there is no unsynced state (i.e. no transactions
    // were executed after the last sealed batch).
    // FIXME: doesn't work w/ snapshot recovery; change to a dedicated DB query?
    let pending_miniblock_number = {
        let (_, last_miniblock_number_included_in_l1_batch) = storage
            .blocks_dal()
            .get_miniblock_range_of_l1_batch(current_l1_batch_number - 1)
            .await
            .unwrap()
            .unwrap();
        last_miniblock_number_included_in_l1_batch + 1
    };
    let pending_miniblock_header = storage
        .blocks_dal()
        .get_miniblock_header(pending_miniblock_number)
        .await
        .unwrap()?;

    tracing::info!("Getting previous batch hash");
    let (previous_l1_batch_hash, _) =
        extractors::wait_for_prev_l1_batch_params(storage, current_l1_batch_number).await;

    tracing::info!("Getting previous miniblock hash");
    let prev_miniblock_hash = storage
        .blocks_dal()
        .get_miniblock_header(pending_miniblock_number - 1)
        .await
        .unwrap()
        .unwrap()
        .hash;

    let base_system_contracts = storage
        .storage_dal()
        .get_base_system_contracts(
            pending_miniblock_header
                .base_system_contracts_hashes
                .bootloader,
            pending_miniblock_header
                .base_system_contracts_hashes
                .default_aa,
        )
        .await;

    tracing::info!("Previous l1_batch_hash: {}", previous_l1_batch_hash);
    Some(l1_batch_params(
        current_l1_batch_number,
        fee_account,
        pending_miniblock_header.timestamp,
        previous_l1_batch_hash,
        pending_miniblock_header.batch_fee_input,
        pending_miniblock_number,
        prev_miniblock_hash,
        base_system_contracts,
        validation_computational_gas_limit,
        pending_miniblock_header
            .protocol_version
            .expect("`protocol_version` must be set for pending miniblock"),
        pending_miniblock_header.virtual_blocks,
        chain_id,
    ))
}

/// Loads the pending L1 block data from the database.
pub(crate) async fn load_pending_batch(
    storage: &mut StorageProcessor<'_>,
    current_l1_batch_number: L1BatchNumber,
    fee_account: Address,
    validation_computational_gas_limit: u32,
    chain_id: L2ChainId,
) -> Option<PendingBatchData> {
    let (system_env, l1_batch_env) = load_l1_batch_params(
        storage,
        current_l1_batch_number,
        fee_account,
        validation_computational_gas_limit,
        chain_id,
    )
    .await?;

    let pending_miniblocks = storage
        .transactions_dal()
        .get_miniblocks_to_reexecute()
        .await
        .unwrap();

    Some(PendingBatchData {
        l1_batch_env,
        system_env,
        pending_miniblocks,
    })
}

/// Cursor of the miniblock / L1 batch progress used by [`StateKeeperIO`](super::StateKeeperIO) implementations.
#[derive(Debug)]
pub(crate) struct IoCursor {
    pub next_miniblock: MiniblockNumber,
    pub prev_miniblock_hash: H256,
    pub prev_miniblock_timestamp: u64,
    pub l1_batch: L1BatchNumber,
}

impl IoCursor {
    /// Loads the cursor from Postgres.
    pub async fn new(storage: &mut StorageProcessor<'_>) -> anyhow::Result<Self> {
        let last_sealed_l1_batch_number = storage
            .blocks_dal()
            .get_sealed_l1_batch_number()
            .await
            .context("Failed getting sealed L1 batch number")?;
        let last_miniblock_header = storage
            .blocks_dal()
            .get_last_sealed_miniblock_header()
            .await
            .context("Failed getting sealed miniblock header")?;

        if let (Some(l1_batch_number), Some(miniblock_header)) =
            (last_sealed_l1_batch_number, &last_miniblock_header)
        {
            Ok(Self {
                next_miniblock: miniblock_header.number + 1,
                prev_miniblock_hash: miniblock_header.hash,
                prev_miniblock_timestamp: miniblock_header.timestamp,
                l1_batch: l1_batch_number + 1,
            })
        } else {
            let snapshot_recovery = storage
                .snapshot_recovery_dal()
                .get_applied_snapshot_status()
                .await
                .context("Failed getting snapshot recovery info")?
                .context("Postgres contains neither blocks nor snapshot recovery info")?;
            let l1_batch =
                last_sealed_l1_batch_number.unwrap_or(snapshot_recovery.l1_batch_number) + 1;

            let (next_miniblock, prev_miniblock_hash, prev_miniblock_timestamp);
            if let Some(miniblock_header) = &last_miniblock_header {
                next_miniblock = miniblock_header.number + 1;
                prev_miniblock_hash = miniblock_header.hash;
                prev_miniblock_timestamp = miniblock_header.timestamp;
            } else {
                next_miniblock = snapshot_recovery.miniblock_number + 1;
                prev_miniblock_hash = snapshot_recovery.miniblock_hash;
                prev_miniblock_timestamp = snapshot_recovery.miniblock_timestamp;
            }

            Ok(Self {
                next_miniblock,
                prev_miniblock_hash,
                prev_miniblock_timestamp,
                l1_batch,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use zksync_dal::ConnectionPool;
    use zksync_types::block::MiniblockHasher;

    use super::*;
    use crate::{
        genesis::{ensure_genesis_state, GenesisParams},
        utils::testonly::{create_miniblock, prepare_empty_recovery_snapshot},
    };

    #[test]
    #[rustfmt::skip] // One-line formatting looks better here.
    fn test_poll_iters() {
        assert_eq!(poll_iters(Duration::from_millis(100), Duration::from_millis(0)), 1);
        assert_eq!(poll_iters(Duration::from_millis(100), Duration::from_millis(100)), 1);
        assert_eq!(poll_iters(Duration::from_millis(100), Duration::from_millis(101)), 2);
        assert_eq!(poll_iters(Duration::from_millis(100), Duration::from_millis(200)), 2);
        assert_eq!(poll_iters(Duration::from_millis(100), Duration::from_millis(201)), 3);
    }

    #[tokio::test]
    async fn creating_io_cursor_with_genesis() {
        let pool = ConnectionPool::test_pool().await;
        let mut storage = pool.access_storage().await.unwrap();
        ensure_genesis_state(&mut storage, L2ChainId::default(), &GenesisParams::mock())
            .await
            .unwrap();

        let cursor = IoCursor::new(&mut storage).await.unwrap();
        assert_eq!(cursor.l1_batch, L1BatchNumber(1));
        assert_eq!(cursor.next_miniblock, MiniblockNumber(1));
        assert_eq!(cursor.prev_miniblock_timestamp, 0);
        assert_eq!(
            cursor.prev_miniblock_hash,
            MiniblockHasher::legacy_hash(MiniblockNumber(0))
        );

        let miniblock = create_miniblock(1);
        storage
            .blocks_dal()
            .insert_miniblock(&miniblock)
            .await
            .unwrap();

        let cursor = IoCursor::new(&mut storage).await.unwrap();
        assert_eq!(cursor.l1_batch, L1BatchNumber(1));
        assert_eq!(cursor.next_miniblock, MiniblockNumber(2));
        assert_eq!(cursor.prev_miniblock_timestamp, miniblock.timestamp);
        assert_eq!(cursor.prev_miniblock_hash, miniblock.hash);
    }

    #[tokio::test]
    async fn creating_io_cursor_with_snapshot_recovery() {
        let pool = ConnectionPool::test_pool().await;
        let mut storage = pool.access_storage().await.unwrap();
        let snapshot_recovery = prepare_empty_recovery_snapshot(&mut storage, 23).await;

        let cursor = IoCursor::new(&mut storage).await.unwrap();
        assert_eq!(cursor.l1_batch, L1BatchNumber(24));
        assert_eq!(
            cursor.next_miniblock,
            snapshot_recovery.miniblock_number + 1
        );
        assert_eq!(
            cursor.prev_miniblock_timestamp,
            snapshot_recovery.miniblock_timestamp
        );
        assert_eq!(cursor.prev_miniblock_hash, snapshot_recovery.miniblock_hash);

        // Add a miniblock so that we have miniblocks (but not an L1 batch) in the storage.
        let miniblock = create_miniblock(snapshot_recovery.miniblock_number.0 + 1);
        storage
            .blocks_dal()
            .insert_miniblock(&miniblock)
            .await
            .unwrap();

        let cursor = IoCursor::new(&mut storage).await.unwrap();
        assert_eq!(cursor.l1_batch, L1BatchNumber(24));
        assert_eq!(cursor.next_miniblock, miniblock.number + 1);
        assert_eq!(cursor.prev_miniblock_timestamp, miniblock.timestamp);
        assert_eq!(cursor.prev_miniblock_hash, miniblock.hash);
    }
}

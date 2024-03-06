mod checkpoint;
mod mode;
mod segment;
mod target;

use crate::{Address, BlockNumber};
pub use checkpoint::PruneCheckpoint;
pub use mode::PruneMode;
pub use segment::{PrunePurpose, PruneSegment, PruneSegmentError};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
pub use target::{PruneModes, MINIMUM_PRUNING_DISTANCE};

/// Configuration for pruning receipts not associated with logs emitted by the specified contracts.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ReceiptsLogPruneConfig(pub BTreeMap<Address, PruneMode>);

impl ReceiptsLogPruneConfig {
    /// Checks if the configuration is empty
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Given the `tip` block number, consolidates the structure so it can easily be queried for
    /// filtering across a range of blocks.
    ///
    /// Example:
    ///
    /// `{ addrA: Before(872), addrB: Before(500), addrC: Distance(128) }`
    ///  
    ///    for `tip: 1000`, gets transformed to a map such as:
    ///
    /// `{ 500: [addrB], 872: [addrA, addrC] }`
    ///
    /// The [`BlockNumber`] key of the new map should be viewed as `PruneMode::Before(block)`, which
    /// makes the previous result equivalent to
    ///
    /// `{ Before(500): [addrB], Before(872): [addrA, addrC] }`
    pub fn group_by_block(
        &self,
        tip: BlockNumber,
        pruned_block: Option<BlockNumber>,
    ) -> Result<BTreeMap<BlockNumber, Vec<&Address>>, PruneSegmentError> {
        let mut map = BTreeMap::new();
        let pruned_block = pruned_block.unwrap_or_default();

        for (address, mode) in self.0.iter() {
            // Getting `None`, means that there is nothing to prune yet, so we need it to include in
            // the BTreeMap (block = 0), otherwise it will be excluded.
            // Reminder that this BTreeMap works as an inclusion list that excludes (prunes) all
            // other receipts.
            //
            // Reminder, that we increment because the [`BlockNumber`] key of the new map should be
            // viewed as `PruneMode::Before(block)`
            let block = (pruned_block + 1).max(
                mode.prune_target_block(tip, PruneSegment::ContractLogs, PrunePurpose::User)?
                    .map(|(block, _)| block)
                    .unwrap_or_default() +
                    1,
            );

            map.entry(block).or_insert_with(Vec::new).push(address)
        }
        Ok(map)
    }

    /// Returns the lowest block where we start filtering logs which use `PruneMode::Distance(_)`.
    pub fn lowest_block_with_distance(
        &self,
        tip: BlockNumber,
        pruned_block: Option<BlockNumber>,
    ) -> Result<Option<BlockNumber>, PruneSegmentError> {
        let pruned_block = pruned_block.unwrap_or_default();
        let mut lowest = None;

        for (_, mode) in self.0.iter() {
            if let PruneMode::Distance(_) = mode {
                if let Some((block, _)) =
                    mode.prune_target_block(tip, PruneSegment::ContractLogs, PrunePurpose::User)?
                {
                    lowest = Some(lowest.unwrap_or(u64::MAX).min(block));
                }
            }
        }

        Ok(lowest.map(|lowest| lowest.max(pruned_block)))
    }
}

/// Progress of pruning.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum PruneProgress {
    /// There is more data to prune.
    HasMoreData(PruneInterruptReason),
    /// Pruning has been finished.
    Finished,
}

/// Reason for interrupting a prune job.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum PruneInterruptReason {
    /// Prune job timed out.
    Timeout,
    /// Limit on the number of deleted segments per prune job was reached.
    LimitSegmentsDeleted,
}

impl PruneInterruptReason {
    /// Returns `true` if reason is timeout.
    pub fn is_timeout(&self) -> bool {
        match self {
            Self::Timeout => true,
            Self::LimitSegmentsDeleted => false,
        }
    }

    /// Returns `true` if reason is reaching limit on deleted segments.
    pub fn is_segment_limit_reached(&self) -> bool {
        match self {
            Self::Timeout => false,
            Self::LimitSegmentsDeleted => true,
        }
    }
}

impl PruneProgress {
    /// Creates new [PruneProgress] that summarises prune job.
    ///
    /// If `done == true`, returns [`PruneProgress::Finished`], otherwise
    /// [PruneProgress::HasMoreData] is returned.
    pub fn new(done: bool, timeout: bool) -> Self {
        if done {
            Self::Finished
        } else if timeout {
            Self::timed_out()
        } else {
            Self::segment_limit_reached()
        }
    }

    /// Returns a new instance of variant [`Finished`](Self::Finished).
    pub const fn finished() -> Self {
        Self::Finished
    }

    /// Returns a new instance of variant [`HasMoreData`](Self::HasMoreData) with
    /// [`PruneInterruptReason::Timeout`].
    pub const fn timed_out() -> Self {
        Self::HasMoreData(PruneInterruptReason::Timeout)
    }

    /// Returns a new instance of variant [`HasMoreData`](Self::HasMoreData) with
    /// [`PruneInterruptReason::LimitSegmentsDeleted`].
    pub const fn segment_limit_reached() -> Self {
        Self::HasMoreData(PruneInterruptReason::LimitSegmentsDeleted)
    }

    /// Returns `true` if prune job is done.
    pub fn is_done(&self) -> bool {
        match self {
            Self::Finished => true,
            Self::HasMoreData(_) => false,
        }
    }

    /// Returns `true` if prune job was interrupted by timeout.
    pub fn is_timed_out(&self) -> bool {
        match self {
            Self::Finished => false,
            Self::HasMoreData(reason) => reason.is_timeout(),
        }
    }

    /// Returns `true` if prune job was interrupted by reaching limit on deleted segments.
    pub fn is_segment_limit_reached(&self) -> bool {
        match self {
            Self::Finished => false,
            Self::HasMoreData(reason) => reason.is_segment_limit_reached(),
        }
    }
}

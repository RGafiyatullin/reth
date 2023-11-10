//! Factory for parallel EVM executor.
use crate::{executor::ParallelExecutor, queue::TransitionQueueStore};
use reth_primitives::ChainSpec;
use reth_provider::{BlockReader, PrunableBlockRangeExecutor, RangeExecutorFactory, StateProvider};
use reth_revm_database::StateProviderDatabase;
use std::sync::Arc;

/// Factory to create parallel executor.
#[derive(Clone, Debug)]
pub struct ParallelExecutorFactory {
    chain_spec: Arc<ChainSpec>,
    queue_store: Arc<TransitionQueueStore>,
}

impl ParallelExecutorFactory {
    /// Create new factory
    pub fn new(chain_spec: Arc<ChainSpec>, queue_store: Arc<TransitionQueueStore>) -> Self {
        Self { chain_spec, queue_store }
    }
}

impl RangeExecutorFactory for ParallelExecutorFactory {
    fn with_provider_and_state<'a, Provider, SP>(
        &'a self,
        provider: Provider,
        sp: SP,
    ) -> Box<dyn PrunableBlockRangeExecutor + 'a>
    where
        Provider: BlockReader + 'a,
        SP: StateProvider + 'a,
    {
        Box::new(
            ParallelExecutor::new(
                provider,
                Arc::clone(&self.chain_spec),
                Arc::clone(&self.queue_store),
                Box::new(StateProviderDatabase::new(sp)),
                None,
            )
            .expect("success"), // TODO:
        )
    }

    fn chain_spec(&self) -> &ChainSpec {
        &self.chain_spec
    }
}
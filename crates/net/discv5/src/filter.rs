//! Predicates to constraint peer lookups.

use alloy_rlp::Decodable;
use dashmap::DashSet;
use derive_more::Constructor;
use itertools::Itertools;
use reth_primitives::{ForkId, MAINNET};

use crate::{IdentifyForkIdKVPair, NetworkRef};

/// Allows users to inject custom filtering rules on which peers to discover.
pub trait FilterDiscovered {
    /// Applies filtering rules on [`Enr`](discv5::Enr) data. Returns [`Ok`](FilterOutcome::Ok) if
    /// peer should be included, otherwise [`Ignore`](FilterOutcome::Ignore).
    fn filter(&self, enr: &discv5::Enr) -> FilterOutcome;

    /// Message for [`FilterOutcome::Ignore`] should specify the reason for filtering out a node
    /// record.
    fn ignore_reason(&self) -> String;
}

/// Outcome of applying filtering rules on node record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilterOutcome {
    /// ENR passes filter rules.
    Ok,
    /// ENR passes filter rules. [`ForkId`] is a by-product of filtering, and is returned to avoid
    /// rlp decoding it twice.
    OkReturnForkId(ForkId),
    /// ENR doesn't pass filter rules, for the given reason.
    Ignore {
        /// Reason for filtering out node record.
        reason: String,
    },
}

impl FilterOutcome {
    /// Returns `true` for [`FilterOutcome::Ok`].
    pub fn is_ok(&self) -> bool {
        matches!(self, FilterOutcome::Ok)
    }
}

/// Filter requiring that peers advertise that they belong to some fork of a certain chain.
#[derive(Debug, Constructor, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MustIncludeChain {
    /// Chain which node record must advertise.
    chain: &'static [u8],
}

impl FilterDiscovered for MustIncludeChain {
    fn filter(&self, enr: &discv5::Enr) -> FilterOutcome {
        if enr.get_raw_rlp(self.chain).is_none() {
            return FilterOutcome::Ignore { reason: self.ignore_reason() }
        }
        FilterOutcome::Ok
    }

    fn ignore_reason(&self) -> String {
        format!("{} fork required", String::from_utf8_lossy(self.chain))
    }
}

impl Default for MustIncludeChain {
    fn default() -> Self {
        Self { chain: NetworkRef::ETH }
    }
}

/// Filter requiring that peers not advertise that they belong to some chains.
#[derive(Debug, Clone, Default)]
pub struct MustNotIncludeChains {
    chains: DashSet<MustIncludeChain>,
}

impl MustNotIncludeChains {
    /// Returns a new instance that disallows node records with a kv-pair that has any of the given
    /// chains as key.
    pub fn new(disallow_chains: &[&'static [u8]]) -> Self {
        let chains = DashSet::with_capacity(disallow_chains.len());
        for chain in disallow_chains {
            _ = chains.insert(MustIncludeChain::new(chain));
        }

        MustNotIncludeChains { chains }
    }
}

impl FilterDiscovered for MustNotIncludeChains {
    fn filter(&self, enr: &discv5::Enr) -> FilterOutcome {
        for chain in self.chains.iter() {
            if matches!(chain.filter(enr), FilterOutcome::Ok) {
                return FilterOutcome::Ignore { reason: self.ignore_reason() }
            }
        }
        if enr.get_raw_rlp(NetworkRef::ETH2).is_some() {
            return FilterOutcome::Ignore { reason: self.ignore_reason() }
        }
        FilterOutcome::Ok
    }

    fn ignore_reason(&self) -> String {
        format!(
            "{} forks not allowed",
            self.chains.iter().map(|chain| String::from_utf8_lossy(chain.chain)).format(",")
        )
    }
}

/// Filter requiring that peers advertise belonging to a certain fork.
#[derive(Debug, Clone)]
pub struct MustIncludeFork {
    /// Filters chain which node record must advertise.
    chain: MustIncludeChain,
    /// Fork which node record must advertise.
    fork_id: ForkId,
}

impl MustIncludeFork {
    /// Returns a new instance.
    pub fn new(chain: &'static [u8], fork_id: ForkId) -> Self {
        Self { chain: MustIncludeChain::new(chain), fork_id }
    }
}

impl FilterDiscovered for MustIncludeFork {
    fn filter(&self, enr: &discv5::Enr) -> FilterOutcome {
        let Some(mut fork_id_bytes) = enr.get_raw_rlp(self.chain.chain) else {
            return FilterOutcome::Ignore { reason: self.chain.ignore_reason() }
        };

        if let Ok(fork_id) = ForkId::decode(&mut fork_id_bytes) {
            if fork_id == self.fork_id {
                return FilterOutcome::OkReturnForkId(fork_id)
            }
        }

        FilterOutcome::Ignore { reason: self.ignore_reason() }
    }

    fn ignore_reason(&self) -> String {
        format!("{} fork {:?} required", String::from_utf8_lossy(self.chain.chain), self.fork_id)
    }
}

impl Default for MustIncludeFork {
    fn default() -> Self {
        Self { chain: MustIncludeChain::new(NetworkRef::ETH), fork_id: MAINNET.latest_fork_id() }
    }
}

#[cfg(test)]
mod tests {
    use alloy_rlp::Bytes;
    use discv5::enr::{CombinedKey, Enr};

    use super::*;

    #[test]
    fn fork_filter() {
        // rig test

        let fork = MAINNET.cancun_fork_id().unwrap();
        let filter = MustIncludeFork::new(b"eth", fork);

        // enr_1 advertises fork configured in filter
        let sk = CombinedKey::generate_secp256k1();
        let enr_1 = Enr::builder()
            .add_value_rlp(NetworkRef::ETH as &[u8], alloy_rlp::encode(fork).into())
            .build(&sk)
            .unwrap();

        // enr_2 advertises an older fork
        let sk = CombinedKey::generate_secp256k1();
        let enr_2 = Enr::builder()
            .add_value_rlp(
                NetworkRef::ETH,
                alloy_rlp::encode(MAINNET.shanghai_fork_id().unwrap()).into(),
            )
            .build(&sk)
            .unwrap();

        // test

        assert!(matches!(filter.filter(&enr_1), FilterOutcome::OkReturnForkId(_)));
        assert!(matches!(filter.filter(&enr_2), FilterOutcome::Ignore { .. }));
    }

    #[test]
    fn must_not_include_chain_filter() {
        // rig test

        let filter = MustNotIncludeChains::new(&[b"eth", b"eth2"]);

        // enr_1 advertises a fork from one of the chains configured in filter
        let sk = CombinedKey::generate_secp256k1();
        let enr_1 = Enr::builder()
            .add_value_rlp(NetworkRef::ETH as &[u8], Bytes::from("cancun"))
            .build(&sk)
            .unwrap();

        // enr_2 advertises a fork from one the other chain configured in filter
        let sk = CombinedKey::generate_secp256k1();
        let enr_2 = Enr::builder()
            .add_value_rlp(NetworkRef::ETH2, Bytes::from("deneb"))
            .build(&sk)
            .unwrap();

        // test

        assert!(matches!(filter.filter(&enr_1), FilterOutcome::Ignore { .. }));
        assert!(matches!(filter.filter(&enr_2), FilterOutcome::Ignore { .. }));
    }
}
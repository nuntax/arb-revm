use eyre::Result;
use revm::primitives::{B256, U256};

use super::{StorageBytes, StorageSpace, Subspace};
use crate::arb_journal::ArbJournal;
use crate::constants::ARBOS_STATE_ADDRESS;

/// Reads the serialized ArbOS chain-config blob directly from state, mirroring Nitro's
/// `arbosState.ChainConfig()`. The chain config lives in the ArbOS account's subspace 7
/// (`chainConfigSubspace`), written at genesis and updatable via `ArbOwner.setChainConfig`, so a
/// node can recover per-chain params (e.g. `MaxCodeSize`) from state alone, independent of how it
/// booted (orbit files or an imported snapshot). `read_slot(slot)` must return the value of
/// `ARBOS_STATE_ADDRESS[slot]`. Returns an empty vec when unset. Mirrors [`StorageBytes::get`]'s
/// layout (slot 0 = length, slots 1.. = 32-byte chunks, last chunk right-aligned).
pub fn read_serialized_chain_config(mut read_slot: impl FnMut(B256) -> U256) -> Vec<u8> {
    let space =
        StorageSpace::new(ARBOS_STATE_ADDRESS).open_subspace_with_key(Subspace::ChainConfig as u8);
    let slot = |key: u64| -> B256 { space.slot_for_u256(U256::from(key)) };

    let len = read_slot(slot(0)).saturating_to::<u64>() as usize;
    if len == 0 {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(len);
    let mut bytes_left = len;
    let mut offset = 1_u64;
    while bytes_left >= 32 {
        out.extend_from_slice(&read_slot(slot(offset)).to_be_bytes::<32>());
        bytes_left -= 32;
        offset += 1;
    }
    if bytes_left > 0 {
        let word = read_slot(slot(offset)).to_be_bytes::<32>();
        out.extend_from_slice(&word[32 - bytes_left..]);
    }
    out
}

/// ArbOS chain-config blob storage.
#[derive(Debug)]
pub struct ChainConfig {
    bytes: StorageBytes,
}

impl ChainConfig {
    pub fn open(storage: &StorageSpace) -> Self {
        Self {
            bytes: StorageBytes::open(storage),
        }
    }

    pub fn get<J: ArbJournal>(&self, journal: &mut J) -> Result<Vec<u8>> {
        self.bytes.get(journal)
    }

    pub fn set<J: ArbJournal>(&self, value: &[u8], journal: &mut J) -> Result<()> {
        self.bytes.set(value, journal)
    }

    pub fn clear<J: ArbJournal>(&self, journal: &mut J) -> Result<()> {
        self.bytes.clear(journal)
    }

    pub fn size<J: ArbJournal>(&self, journal: &mut J) -> Result<u64> {
        self.bytes.size(journal)
    }
}

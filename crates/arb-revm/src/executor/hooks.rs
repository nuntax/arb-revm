use crate::{constants::ARBOS_ACTS_ADDRESS, executor::contract::ArbExecutionInput};
use alloy_core::{sol, sol_types::SolCall};
use revm::primitives::{Address, Bytes};

sol! {
    interface ArbosActs {
        function startBlock(
            uint256 l1BaseFee,
            uint64 l1BlockNumber,
            uint64 l2BlockNumber,
            uint64 timeLastBlock
        ) external;
    }
}

/// Derived values for start-of-block prelude wiring.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArbStartBlockDerived {
    pub l2_block_number: u64,
    pub time_last_block: u64,
}

/// Descriptor for an internal prelude action emitted by execution hooks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArbSystemCall {
    pub caller: Address,
    pub target: Address,
    pub data: Bytes,
}

/// Hook extension points for message execution.
pub trait ArbExecutionHooks {
    /// Optional start-of-block prelude action.
    ///
    /// Return `None` to skip start-block prelude execution.
    fn start_block_prelude(
        &self,
        input: &ArbExecutionInput,
        derived: ArbStartBlockDerived,
    ) -> Option<ArbSystemCall>;
}

/// Default hook set: ArbOSActs `startBlock` as a typed internal call payload.
#[derive(Clone, Copy, Debug, Default)]
pub struct DefaultArbExecutionHooks;

impl ArbExecutionHooks for DefaultArbExecutionHooks {
    fn start_block_prelude(
        &self,
        input: &ArbExecutionInput,
        derived: ArbStartBlockDerived,
    ) -> Option<ArbSystemCall> {
        let message = &input.message;
        let l1_base_fee =
            alloy_core::primitives::U256::from_limbs(*message.l1_base_fee_wei.as_limbs());

        let data = ArbosActs::startBlockCall::new((
            l1_base_fee,
            message.l1_block_number,
            derived.l2_block_number,
            derived.time_last_block,
        ))
        .abi_encode();

        Some(ArbSystemCall {
            caller: ARBOS_ACTS_ADDRESS,
            target: ARBOS_ACTS_ADDRESS,
            data: Bytes::from(data),
        })
    }
}

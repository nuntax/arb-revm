use revm::primitives::{Address, address};

/// ArbOS system actor address used for internal calls in Nitro.
pub const ARBOS_ACTS_ADDRESS: Address = address!("0x00000000000000000000000000000000000A4B05");

/// Root ArbOS state account used by Nitro.
pub const ARBOS_STATE_ADDRESS: Address = address!("0xA4B05FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF");

/// Sequencer batch poster sentinel account used by ArbOS pricing.
pub const BATCH_POSTER_ADDRESS: Address = address!("0xA4B000000000000000000073657175656e636572");

/// ArbOS L1 pricer funds pool account (Nitro `types.L1PricerFundsPoolAddress`).
pub const L1_PRICER_FUNDS_POOL_ADDRESS: Address =
    address!("0xA4B00000000000000000000000000000000000f6");

/// Address aliasing offset applied to retryable/L1-originated senders.
pub const ADDRESS_ALIAS_OFFSET_HEX: &str = "1111000000000000000000000000000000001111";

/// Nitro typed transaction discriminator for ArbOS internal transactions.
pub const ARBITRUM_INTERNAL_TX_TYPE: u8 = 0x6a;

/// Nitro typed transaction discriminator for L1->L2 ETH deposit transactions.
pub const ARBITRUM_DEPOSIT_TX_TYPE: u8 = 0x64;

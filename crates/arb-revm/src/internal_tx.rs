use crate::{api::exec::ArbContextTr, storage::ArbosState};
use revm::{
    Database as _,
    context_interface::{Block, ContextTr, JournalTr, Transaction},
    primitives::{Address, B256, Bytes, U256, keccak256},
};

const START_BLOCK_SELECTOR_TEXT: &[u8] = b"startBlock(uint256,uint64,uint64,uint64)";
const BATCH_POSTING_REPORT_SELECTOR_TEXT: &[u8] =
    b"batchPostingReport(uint256,address,uint64,uint64,uint256)";
const BATCH_POSTING_REPORT_V2_SELECTOR_TEXT: &[u8] =
    b"batchPostingReportV2(uint256,address,uint64,uint64,uint64,uint64,uint256)";

const START_BLOCK_CALLDATA_WORDS: usize = 4;
const BATCH_POSTING_REPORT_CALLDATA_WORDS: usize = 5;
const BATCH_POSTING_REPORT_V2_CALLDATA_WORDS: usize = 7;

const ABI_WORD_SIZE: usize = 32;
const SELECTOR_SIZE: usize = 4;

const ARBOS_VERSION_BATCH_REPORT_V2_FLOOR_GAS: u64 = 50;
const FLOOR_GAS_ADDITIONAL_TOKENS: u64 = 172;

const TX_DATA_ZERO_GAS: u64 = 4;
const TX_DATA_NON_ZERO_GAS_EIP2028: u64 = 16;
const KECCAK256_GAS: u64 = 30;
const KECCAK256_WORD_GAS: u64 = 6;
const SSTORE_SET_GAS_EIP2200: u64 = 20_000;
const TX_GAS: u64 = 21_000;

pub(crate) fn start_block_selector() -> [u8; 4] {
    let hash = keccak256(START_BLOCK_SELECTOR_TEXT);
    [hash[0], hash[1], hash[2], hash[3]]
}

pub(crate) fn batch_posting_report_selector() -> [u8; 4] {
    let hash = keccak256(BATCH_POSTING_REPORT_SELECTOR_TEXT);
    [hash[0], hash[1], hash[2], hash[3]]
}

pub(crate) fn batch_posting_report_v2_selector() -> [u8; 4] {
    let hash = keccak256(BATCH_POSTING_REPORT_V2_SELECTOR_TEXT);
    [hash[0], hash[1], hash[2], hash[3]]
}

pub(crate) fn apply_internal_tx<CTX: ArbContextTr>(ctx: &mut CTX) -> Result<(), String> {
    let input = ctx.tx().input().clone();
    if input.len() < SELECTOR_SIZE {
        return Err("[ARBITRUM] internal tx calldata shorter than selector".into());
    }

    let selector =
        <[u8; 4]>::try_from(&input[..SELECTOR_SIZE]).expect("selector slice length is fixed");

    if selector == start_block_selector() {
        return apply_start_block(ctx, &input);
    }
    if selector == batch_posting_report_selector() {
        return apply_batch_posting_report(ctx, &input);
    }
    if selector == batch_posting_report_v2_selector() {
        return apply_batch_posting_report_v2(ctx, &input);
    }

    Err(format!(
        "[ARBITRUM] unsupported internal tx selector 0x{}",
        hex_encode(&selector)
    ))
}

fn apply_start_block<CTX: ArbContextTr>(ctx: &mut CTX, input: &Bytes) -> Result<(), String> {
    let (_l1_base_fee, mut l1_block_number, l2_block_number, mut time_last_block) =
        decode_start_block_calldata(input)?;

    let current_l2_block_number: u64 = ctx
        .block()
        .number()
        .try_into()
        .map_err(|_| "[ARBITRUM] block.number does not fit in u64".to_string())?;
    if l2_block_number != current_l2_block_number {
        return Err(format!(
            "[ARBITRUM] startBlock l2BlockNumber mismatch: got {l2_block_number}, expected {current_l2_block_number}"
        ));
    }

    let prev_hash = if current_l2_block_number == 0 {
        B256::ZERO
    } else {
        ctx.journal_mut()
            .db_mut()
            .block_hash(current_l2_block_number - 1)
            .map_err(|err| format!("[ARBITRUM] failed to read parent block hash: {err}"))?
    };

    let arbos_state = ArbosState::open();
    let journal = ctx.journal_mut();
    let arbos_version = arbos_state
        .arbos_version
        .get(journal)
        .map_err(|err| format!("[ARBITRUM] failed to read ArbOS version: {err}"))?;

    // Version compatibility shims mirror Nitro behavior.
    if arbos_version < 3 {
        time_last_block = l2_block_number;
    }
    if arbos_version < 8 {
        l1_block_number = l1_block_number.saturating_add(1);
    }

    let old_l1_block_number = arbos_state
        .block_hashes
        .l1_block_number(journal)
        .map_err(|err| format!("[ARBITRUM] failed to read ArbOS L1 block number: {err}"))?;
    if l1_block_number > old_l1_block_number {
        arbos_state
            .block_hashes
            .record_new_l1_block(l1_block_number - 1, prev_hash, arbos_version, journal)
            .map_err(|err| format!("[ARBITRUM] failed to record ArbOS L1 block hash: {err}"))?;
    }

    arbos_state
        .l2_pricing
        .update_pricing_model(time_last_block, journal)
        .map_err(|err| format!("[ARBITRUM] failed to update ArbOS L2 pricing model: {err}"))?;

    // TODO(parity): handle retryable reaping and ArbOS upgrade checks from Nitro's
    // ApplyInternalTxUpdate(StartBlock) path.
    Ok(())
}

fn apply_batch_posting_report<CTX: ArbContextTr>(
    ctx: &mut CTX,
    input: &Bytes,
) -> Result<(), String> {
    let (batch_timestamp, batch_poster_address, _batch_number, batch_data_gas, l1_base_fee_wei) =
        decode_batch_posting_report_calldata(input)?;
    let batch_timestamp: u64 = batch_timestamp.try_into().map_err(|_| {
        "[ARBITRUM] batchPostingReport batchTimestamp does not fit in u64".to_string()
    })?;
    let current_time: u64 = ctx
        .block()
        .timestamp()
        .try_into()
        .map_err(|_| "[ARBITRUM] block.timestamp does not fit in u64".to_string())?;

    let arbos_state = ArbosState::open();
    let journal = ctx.journal_mut();
    let arbos_version = arbos_state
        .arbos_version
        .get(journal)
        .map_err(|err| format!("[ARBITRUM] failed to read ArbOS version: {err}"))?;

    let per_batch_gas = arbos_state
        .l1_pricing
        .per_batch_gas_cost
        .get(journal)
        .map_err(|err| format!("[ARBITRUM] failed to read ArbOS perBatchGasCost: {err}"))?;
    let gas_spent = signed_i64_to_u64_floor_zero(per_batch_gas).saturating_add(batch_data_gas);
    let wei_spent = l1_base_fee_wei
        .checked_mul(U256::from(gas_spent))
        .unwrap_or(U256::MAX);

    arbos_state
        .l1_pricing
        .update_for_batch_poster_spending(
            arbos_version,
            batch_timestamp,
            current_time,
            batch_poster_address,
            wei_spent,
            l1_base_fee_wei,
            journal,
        )
        .map_err(|err| format!("[ARBITRUM] failed to apply batchPostingReport: {err}"))?;
    Ok(())
}

fn apply_batch_posting_report_v2<CTX: ArbContextTr>(
    ctx: &mut CTX,
    input: &Bytes,
) -> Result<(), String> {
    let (
        batch_timestamp,
        batch_poster_address,
        _batch_number,
        batch_calldata_length,
        batch_calldata_non_zeros,
        batch_extra_gas,
        l1_base_fee_wei,
    ) = decode_batch_posting_report_v2_calldata(input)?;

    let batch_timestamp: u64 = batch_timestamp.try_into().map_err(|_| {
        "[ARBITRUM] batchPostingReportV2 batchTimestamp does not fit in u64".to_string()
    })?;
    let current_time: u64 = ctx
        .block()
        .timestamp()
        .try_into()
        .map_err(|_| "[ARBITRUM] block.timestamp does not fit in u64".to_string())?;

    let arbos_state = ArbosState::open();
    let journal = ctx.journal_mut();
    let arbos_version = arbos_state
        .arbos_version
        .get(journal)
        .map_err(|err| format!("[ARBITRUM] failed to read ArbOS version: {err}"))?;

    let per_batch_gas = arbos_state
        .l1_pricing
        .per_batch_gas_cost
        .get(journal)
        .map_err(|err| format!("[ARBITRUM] failed to read ArbOS perBatchGasCost: {err}"))?;

    let mut gas_spent =
        legacy_batch_cost_for_stats(batch_calldata_length, batch_calldata_non_zeros)
            .saturating_add(batch_extra_gas)
            .saturating_add(signed_i64_to_u64_floor_zero(per_batch_gas));

    if arbos_version >= ARBOS_VERSION_BATCH_REPORT_V2_FLOOR_GAS {
        let gas_floor_per_token = arbos_state
            .l1_pricing
            .gas_floor_per_token
            .get(journal)
            .map_err(|err| format!("[ARBITRUM] failed to read ArbOS gasFloorPerToken: {err}"))?;
        let floor_tokens = batch_calldata_length
            .saturating_add(batch_calldata_non_zeros.saturating_mul(3))
            .saturating_add(FLOOR_GAS_ADDITIONAL_TOKENS);
        let floor_gas_spent = gas_floor_per_token
            .saturating_mul(floor_tokens)
            .saturating_add(TX_GAS);
        if floor_gas_spent > gas_spent {
            gas_spent = floor_gas_spent;
        }
    }

    let wei_spent = l1_base_fee_wei
        .checked_mul(U256::from(gas_spent))
        .unwrap_or(U256::MAX);
    arbos_state
        .l1_pricing
        .update_for_batch_poster_spending(
            arbos_version,
            batch_timestamp,
            current_time,
            batch_poster_address,
            wei_spent,
            l1_base_fee_wei,
            journal,
        )
        .map_err(|err| format!("[ARBITRUM] failed to apply batchPostingReportV2: {err}"))?;
    Ok(())
}

fn decode_start_block_calldata(input: &[u8]) -> Result<(U256, u64, u64, u64), String> {
    if input.len() != SELECTOR_SIZE + (START_BLOCK_CALLDATA_WORDS * ABI_WORD_SIZE) {
        return Err(format!(
            "[ARBITRUM] invalid startBlock calldata length {}, expected {}",
            input.len(),
            SELECTOR_SIZE + (START_BLOCK_CALLDATA_WORDS * ABI_WORD_SIZE)
        ));
    }

    let words = &input[SELECTOR_SIZE..];
    let l1_base_fee = word_to_u256(&words[0..ABI_WORD_SIZE]);
    let l1_block_number = word_to_u64(&words[ABI_WORD_SIZE..ABI_WORD_SIZE * 2]);
    let l2_block_number = word_to_u64(&words[ABI_WORD_SIZE * 2..ABI_WORD_SIZE * 3]);
    let time_last_block = word_to_u64(&words[ABI_WORD_SIZE * 3..ABI_WORD_SIZE * 4]);

    Ok((
        l1_base_fee,
        l1_block_number,
        l2_block_number,
        time_last_block,
    ))
}

fn decode_batch_posting_report_calldata(
    input: &[u8],
) -> Result<(U256, Address, u64, u64, U256), String> {
    if input.len() != SELECTOR_SIZE + (BATCH_POSTING_REPORT_CALLDATA_WORDS * ABI_WORD_SIZE) {
        return Err(format!(
            "[ARBITRUM] invalid batchPostingReport calldata length {}, expected {}",
            input.len(),
            SELECTOR_SIZE + (BATCH_POSTING_REPORT_CALLDATA_WORDS * ABI_WORD_SIZE)
        ));
    }

    let words = &input[SELECTOR_SIZE..];
    let batch_timestamp = word_to_u256(&words[0..ABI_WORD_SIZE]);
    let batch_poster_address = word_to_address(&words[ABI_WORD_SIZE..ABI_WORD_SIZE * 2]);
    let batch_number = word_to_u64(&words[ABI_WORD_SIZE * 2..ABI_WORD_SIZE * 3]);
    let batch_data_gas = word_to_u64(&words[ABI_WORD_SIZE * 3..ABI_WORD_SIZE * 4]);
    let l1_base_fee_wei = word_to_u256(&words[ABI_WORD_SIZE * 4..ABI_WORD_SIZE * 5]);

    Ok((
        batch_timestamp,
        batch_poster_address,
        batch_number,
        batch_data_gas,
        l1_base_fee_wei,
    ))
}

fn decode_batch_posting_report_v2_calldata(
    input: &[u8],
) -> Result<(U256, Address, u64, u64, u64, u64, U256), String> {
    if input.len() != SELECTOR_SIZE + (BATCH_POSTING_REPORT_V2_CALLDATA_WORDS * ABI_WORD_SIZE) {
        return Err(format!(
            "[ARBITRUM] invalid batchPostingReportV2 calldata length {}, expected {}",
            input.len(),
            SELECTOR_SIZE + (BATCH_POSTING_REPORT_V2_CALLDATA_WORDS * ABI_WORD_SIZE)
        ));
    }

    let words = &input[SELECTOR_SIZE..];
    let batch_timestamp = word_to_u256(&words[0..ABI_WORD_SIZE]);
    let batch_poster_address = word_to_address(&words[ABI_WORD_SIZE..ABI_WORD_SIZE * 2]);
    let batch_number = word_to_u64(&words[ABI_WORD_SIZE * 2..ABI_WORD_SIZE * 3]);
    let batch_calldata_length = word_to_u64(&words[ABI_WORD_SIZE * 3..ABI_WORD_SIZE * 4]);
    let batch_calldata_non_zeros = word_to_u64(&words[ABI_WORD_SIZE * 4..ABI_WORD_SIZE * 5]);
    let batch_extra_gas = word_to_u64(&words[ABI_WORD_SIZE * 5..ABI_WORD_SIZE * 6]);
    let l1_base_fee_wei = word_to_u256(&words[ABI_WORD_SIZE * 6..ABI_WORD_SIZE * 7]);

    Ok((
        batch_timestamp,
        batch_poster_address,
        batch_number,
        batch_calldata_length,
        batch_calldata_non_zeros,
        batch_extra_gas,
        l1_base_fee_wei,
    ))
}

fn word_to_u256(word: &[u8]) -> U256 {
    let bytes: [u8; ABI_WORD_SIZE] =
        <[u8; ABI_WORD_SIZE]>::try_from(word).expect("ABI word is always 32 bytes");
    U256::from_be_bytes(bytes)
}

fn word_to_address(word: &[u8]) -> Address {
    Address::from_slice(&word[ABI_WORD_SIZE - 20..ABI_WORD_SIZE])
}

fn word_to_u64(word: &[u8]) -> u64 {
    let tail: [u8; 8] = <[u8; 8]>::try_from(&word[ABI_WORD_SIZE - 8..ABI_WORD_SIZE])
        .expect("ABI word tail is always 8 bytes");
    u64::from_be_bytes(tail)
}

fn legacy_batch_cost_for_stats(length: u64, non_zeros: u64) -> u64 {
    let zeros = length.saturating_sub(non_zeros);
    let calldata_gas = TX_DATA_ZERO_GAS
        .saturating_mul(zeros)
        .saturating_add(TX_DATA_NON_ZERO_GAS_EIP2028.saturating_mul(non_zeros));
    let keccak_words = words_for_bytes(length);
    calldata_gas
        .saturating_add(KECCAK256_GAS)
        .saturating_add(keccak_words.saturating_mul(KECCAK256_WORD_GAS))
        .saturating_add(2_u64.saturating_mul(SSTORE_SET_GAS_EIP2200))
}

fn words_for_bytes(byte_len: u64) -> u64 {
    byte_len.saturating_add(31) / 32
}

fn signed_i64_to_u64_floor_zero(value: i64) -> u64 {
    if value <= 0 { 0 } else { value as u64 }
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

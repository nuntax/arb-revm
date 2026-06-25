# Stage A handoff — arb-alloy `reth` feature (NodePrimitives)

**Crate to edit:** `arb-alloy/crates/consensus` (`arb-alloy-consensus`). **Goal:** make arb-alloy's primitives satisfy
reth v2.0.0's `NodePrimitives` so our reth EVM/executor (Stage B+) can use them. **Task #31.**

> Read first: `docs/arb-reth-roadmap.md` (architecture, the op-alloy→op-reth pattern we mirror). This file is the
> standalone build spec for Stage A; you should not need to re-discover anything below.

## The one idea that makes this small

reth's `SignedTransaction` and `Receipt` are **blanket-implemented** for any type that satisfies their supertrait
bounds (see `reth-primitives-traits-0.1.1/src/transaction/signed.rs:123` and `.../receipt.rs:36`). Ethereum's
`EthPrimitives` writes **zero** explicit `SignedTransaction`/`Receipt`/`Block` impls — it just binds alloy types that
already meet the bounds (`reth` git checkout `crates/ethereum/primitives/src/lib.rs:1-52`):

```rust
pub struct EthPrimitives;
impl reth_primitives_traits::NodePrimitives for EthPrimitives {
    type Block       = alloy_consensus::Block<TransactionSigned>;
    type BlockHeader = alloy_consensus::Header;
    type BlockBody   = alloy_consensus::BlockBody<TransactionSigned>;
    type SignedTx    = alloy_consensus::EthereumTxEnvelope<TxEip4844>;
    type Receipt     = EthereumReceipt<TxType>;
}
```

**So Stage A = (1) make `ArbTxEnvelope` and `ArbReceiptEnvelope<Log>` satisfy the supertrait bounds, then (2) bind an
`ArbPrimitives` struct exactly like `EthPrimitives`.** `ArbBlock`/`ArbBlockBody` come free from alloy's blanket
`reth_primitives_traits::Block` impl on `alloy_consensus::Block<T,H>` (`block/mod.rs:206`) once `SignedTx` is
satisfied. You will mostly *chase compiler "trait bound not satisfied" errors and fill them*, not author big impls.

## Target trait bounds (reth-primitives-traits 0.1.1, crates.io)

**`SignedTransaction`** (`src/transaction/signed.rs:27`) supertraits — `ArbTxEnvelope` must satisfy ALL:
`Send + Sync + Unpin + Clone + Debug + PartialEq + Eq + Hash + alloy_rlp::Encodable + alloy_rlp::Decodable +
Encodable2718 + Decodable2718 + alloy_consensus::Transaction + MaybeSerde + InMemorySize + SignerRecoverable +
TxHashRef + IsTyped2718`.

**`Receipt`** (`src/receipt.rs:17`) supertraits — `ArbReceiptEnvelope<Log>` must satisfy ALL:
`Send + Sync + Unpin + Clone + Debug + TxReceipt<Log = alloy_primitives::Log> + RlpEncodableReceipt +
RlpDecodableReceipt + Encodable + Decodable + Eip2718EncodableReceipt + Typed2718 + MaybeSerde + InMemorySize`.
(Note: no `Eq`/`Hash` required for receipts.)

**`NodePrimitives`** (`src/node.rs:9`):
```rust
pub trait NodePrimitives: Send + Sync + Unpin + Clone + Default + Debug + PartialEq + Eq + 'static {
    type Block:       FullBlock<Header = Self::BlockHeader, Body = Self::BlockBody>;
    type BlockHeader: FullBlockHeader;                                  // alloy_consensus::Header qualifies
    type BlockBody:   FullBlockBody<Transaction = Self::SignedTx, OmmerHeader = Self::BlockHeader>;
    type SignedTx:    FullSignedTx;                                     // = SignedTransaction + MaybeCompact
    type Receipt:     FullReceipt;                                      // = Receipt + MaybeCompact
}
```
`MaybeCompact` is **blanket-satisfied when the reth `reth-codec`/compact feature is OFF** (it is, for Stage A) — do
NOT implement `Compact` now (that's Stage D / DB persistence).

## arb-alloy current state (what's there vs. the gaps)

Types (all in `arb-alloy/crates/consensus/src/`):
- `ArbTxEnvelope` (`transactions/mod.rs:37`) — `#[derive(Debug, Clone, TransactionEnvelope)]`. Already impls (via the
  `TransactionEnvelope` macro): `alloy_consensus::Transaction`, `Typed2718`, `Encodable2718`, `Decodable2718`; plus
  hand-written `TxHashRef` (`:164`) and `SignerRecoverable` (`:171`, **k256-gated**). Variants incl. Deposit 0x64,
  SubmitRetryable 0x69, Unsigned 0x65, Contract 0x66, Retry 0x68, Internal 0x6a.
- `ArbReceiptEnvelope<T=Log>` (`receipt.rs:84`) — `#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]`.
  Impls `TxReceipt`, `Typed2718`, `Encodable`, `Decodable`, `Encodable2718`, `Decodable2718`. Inner `ArbReceipt<T>`
  (`:22`) holds `gas_used_for_l1` and impls `RlpEncodableReceipt`/`RlpDecodableReceipt`.
- `ArbHeaderInfo` (`header.rs:29`) — header extra_data/mix_hash codec (send_root, l1_block_number,
  arbos_format_version). Not needed for Stage A, but it's how Stage B/D derive the spec/L1 block.
- No reth deps, no Block type, no NodePrimitives. `Cargo.toml` features: `default=["std"]`, `std`, `serde`, `k256`.

**Gap table** (what to add to clear the bounds):

| bound | `ArbTxEnvelope` | `ArbReceiptEnvelope` | action |
|---|---|---|---|
| `PartialEq, Eq, Hash` | ❌ missing | ✅ has Eq/PartialEq (Hash N/A) | add `PartialEq, Eq, Hash` derives to `ArbTxEnvelope` (+ ensure inner `TxDeposit`/`SubmitRetryableTx`/… derive them) |
| `alloy_rlp::Encodable/Decodable` | likely only 2718 | ✅ has Encodable/Decodable | if missing on tx envelope, impl by delegating to the 2718 network encoding (mirror alloy `EthereumTxEnvelope`) |
| `InMemorySize` | ❌ | ❌ | impl for both envelopes — delegate to inner (`reth-primitives-traits` already impls `InMemorySize` for alloy tx/receipt types; a match-arm `.size()` sum, or its derive) |
| `MaybeSerde` | ⚠️ no serde derives | ✅ serde derives | keep reth-primitives-traits **serde feature OFF** (then MaybeSerde is trivially satisfied), OR add `serde` derives to `ArbTxEnvelope`. Prefer feature-off for Stage A. |
| `IsTyped2718` | verify | n/a | usually blanket from `Typed2718`; if not, impl the marker |
| `SignerRecoverable` | ✅ but k256-gated | n/a | the `reth` feature must enable `k256` |
| `Eip2718EncodableReceipt`/`RlpEncodableReceipt`/`RlpDecodableReceipt` | n/a | verify on the **envelope** (currently confirmed on inner `ArbReceipt`) | if only on inner, add envelope-level impls delegating per-variant |

## Steps

1. **Cargo.toml** (`crates/consensus/Cargo.toml`): add
   ```toml
   [dependencies]
   reth-primitives-traits = { version = "0.1.1", default-features = false, optional = true }
   [features]
   reth = ["dep:reth-primitives-traits", "k256"]
   ```
   Version `0.1.1` matches what `reth-evm` v2.0.0 already resolves (verified in `Cargo.lock`); cargo unifies to the
   same crate instance, so our `NodePrimitives` impl is the one reth's `ConfigureEvm::Primitives` expects. Keep
   reth-primitives-traits' `serde` feature OFF.

2. **Derives on `ArbTxEnvelope`** (`transactions/mod.rs:37`): `#[derive(Debug, Clone, PartialEq, Eq, Hash, TransactionEnvelope)]`.
   Fix any inner type (`TxDeposit`, `SubmitRetryableTx`, `TxUnsigned`, `TxContract`, `TxRetry`, `ArbInternalTx`) that
   doesn't already derive `PartialEq, Eq, Hash`. (Ethereum/alloy inner tx types already do.)

3. **New module** `crates/consensus/src/reth.rs`, gated `#![cfg(feature = "reth")]`, exported from `lib.rs` as
   `#[cfg(feature = "reth")] pub mod reth;`:
   ```rust
   pub type ArbBlock     = alloy_consensus::Block<crate::ArbTxEnvelope>;       // Header defaults to alloy Header
   pub type ArbBlockBody = alloy_consensus::BlockBody<crate::ArbTxEnvelope>;

   #[derive(Debug, Clone, Default, PartialEq, Eq)]
   pub struct ArbPrimitives;

   impl reth_primitives_traits::NodePrimitives for ArbPrimitives {
       type Block       = ArbBlock;
       type BlockHeader = alloy_consensus::Header;
       type BlockBody   = ArbBlockBody;
       type SignedTx    = crate::ArbTxEnvelope;
       type Receipt     = crate::ArbReceiptEnvelope; // <Log> default — TxReceipt<Log = Log> ✓
   }
   ```

4. **Fill supertrait gaps** the compiler reports (InMemorySize, any missing Encodable/Decodable/Eip2718Receipt,
   IsTyped2718). Each is a small delegating impl; the receipt envelope is already ~90% there.

5. **Static assertion test** (`crates/consensus/src/reth.rs` under `#[cfg(test)]` or a `tests/` file):
   ```rust
   fn _assert_node_primitives<T: reth_primitives_traits::NodePrimitives>() {}
   fn _assert_signed_tx<T: reth_primitives_traits::SignedTransaction>() {}
   #[test] fn arb_primitives_satisfies_reth() {
       _assert_node_primitives::<ArbPrimitives>();
       _assert_signed_tx::<crate::ArbTxEnvelope>();
   }
   ```

6. **Wire into the workspace:** in `arb_revm/crates/arb-reth-evm/Cargo.toml`, enable the feature:
   `arb-alloy-consensus = { path = "...", default-features = false, features = ["reth"] }`. Confirm the whole
   workspace still builds.

## Validation (exit criteria)

```
# standalone, with feature
cargo check -p arb-alloy-consensus --features reth --manifest-path arb-alloy/Cargo.toml
# the static assertion compiles+passes
cargo test  -p arb-alloy-consensus --features reth --manifest-path arb-alloy/Cargo.toml arb_primitives_satisfies_reth
# in the reth workspace (unified alloy 1.8.3 + reth-primitives-traits 0.1.1)
cargo check -p arb-reth-evm --manifest-path arb_revm/Cargo.toml
```
Stage A is **done** when `ArbPrimitives: NodePrimitives` compiles in-workspace (the static assertion is the proof).

## Gotchas / scope

- **k256 is mandatory** under `reth` (SignerRecoverable is not optional for reth). Already wired in step 1's feature.
- **Do NOT impl `Compact`** / enable reth's compact codec now — `MaybeCompact` is satisfied without it. DB-storage
  `Compact` impls are a Stage D concern.
- **Receipt generic:** bind `ArbReceiptEnvelope` (i.e. `<Log>`), not `<RpcLog>` — `Receipt` requires
  `TxReceipt<Log = alloy_primitives::Log>`.
- **Header:** reuse `alloy_consensus::Header` (arb-alloy already does; Arbitrum data rides in extra_data/mix_hash via
  `ArbHeaderInfo`). No custom header type.
- **If a duplicate `reth-primitives-traits` appears** (two versions in the tree → "trait not satisfied" despite a
  correct impl), align our dep exactly to the version `reth-evm` uses (`grep -A2 'name = "reth-primitives-traits"'
  arb_revm/Cargo.lock`).
- **Out of scope:** any execution logic, EvmFactory/Evm, BlockExecutor, ArbOS hooks — those are Stages B/C. Stage A
  is *only* the primitives wiring.

## Why this is safe to do in arb-alloy (not a separate crate)
Orphan rule: `InMemorySize`/`Encodable` impls are foreign-trait-on-local-type → must live in the crate defining the
type (arb-alloy-consensus). `NodePrimitives for ArbPrimitives` is local-type → also fine here. The `reth` feature
keeps arb-alloy reth-agnostic by default, mirroring op-alloy (which stays reth-free) while honoring "add to arb-alloy
freely." `SignedTransaction`/`Receipt`/`Block` need no explicit impls — reth's blanket impls cover them once bounds
are met.

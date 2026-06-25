# Stage F handoff — L1 inbox derivation (`arb-reth-derive`)

**The moat.** Decode Arbitrum L1 sequencer-inbox batches into the canonical `MessageWithMetadata` stream — the half
`arbitrum-reth` does not have. **Task #29 (Phase 3).** First milestone = the **calldata happy path** (no blob, no
DAS); the brotli/segment/multiplexer core is identical for blob batches, so this de-risks everything.

## Crate layout

- **`arb-reth-derive`** (NEW, in `arb_revm` workspace `crates/`): pure decoding, no network I/O — `batch bytes →
  Vec<MessageWithMetadata>`. Deps: `arb-sequencer-network` (message types), `nitro/crates/brotli` (the same C brotli
  arb_revm already links — guarantees decompression parity), `alloy-primitives`, `alloy-rlp`.
- **Add to `arb-sequencer-network`** (the types exist as feed/JSON DTOs — add the Nitro **binary wire format**):
  `L1IncomingMessage::serialize()/parse()` (113-byte header, §5) and `MessageWithMetadata::hash()` (RLP+keccak, §6).
  These are shared by the derive path and the Stage G feed path, so they belong on the types.
- **Data acquisition stays separate** (a later `l1source` adapter): SequencerInbox event scan + calldata/blob fetch +
  delayed inbox. For the first milestone use a **committed test-vector fixture** (one real batch's bytes), so the
  decoder is offline + unit-testable.

## Decode pipeline (calldata happy path)
```
L1 tx calldata[4:]  → ABI-unpack addSequencerL2BatchFromOrigin0 → raw batch bytes
  bytes[0..40]  : 5× big-endian u64 header (§2)
  bytes[40]     : flag byte (§2)
    0x00 (brotli) → arbcompress.Decompress(bytes[41:]) → RLP list of [][]byte segments (§3)
       walk segments (multiplexer Pop, §4):
         kind 3/4 (AdvanceTimestamp/AdvanceL1Block): add RLP-u64 delta to running ts/block
         kind 0/1 (L2Message / L2MessageBrotli):     emit MessageWithMetadata (Poster = BatchPoster)
         kind 2   (DelayedMessages):                  pull from delayed inbox, bump delayedMessagesRead
       clamp ts/block to header [Min,Max] bounds before each emit
```

---

## Byte-format reference (from Nitro source — authoritative)

### §1 — SequencerBatchDelivered event + data acquisition
ABI (`contracts/src/bridge/ISequencerInbox.sol:28`): topics `[sig, batchSeqNum(uint256), beforeAcc, afterAcc]`;
data `[delayedAcc(32), afterDelayedMessagesRead(uint256), timeBounds(4×u64 packed), dataLocation(uint8→32)]`.
`dataLocation` enum: **TxInput=0, SeparateBatchEvent=1, NoData=2, Blob=3**.
- **Calldata** (`arbnode/sequencer_inbox.go:93`): tx calldata, skip 4-byte selector, ABI-unpack
  `addSequencerL2BatchFromOrigin0`, take `args["data"] ([]byte)`.
- SeparateBatchEvent (`:108`): `SequencerBatchData(batchSeqNum indexed, bytes data)` in same tx.
- Blob (`:135`): `tx.BlobHashes()` → prepend `0x50` + concat 32-byte versioned hashes (sidecar fetch needed).
- NoData (`:132`): force-inclusion, nil.

### §2 — Batch payload header (40 bytes) + flag (`arbstate/inbox.go:87`)
Five **big-endian u64**: `[0]MinTimestamp [8]MaxTimestamp [16]MinL1Block [24]MaxL1Block [32]AfterDelayedMessages`.
`payload = data[40:]`. Flag = `payload[0]` (`daprovider/util.go`):
`0x00` Brotli · `0x01` DACert · `0x08` AnyTrustTree · `0x20` Zeroheavy · `0x40` L1Authenticated · `0x50` BlobHashes
(`0x40|0x10`) · `0x80` AnyTrust.
Brotli (`inbox.go:174`): `arbcompress.Decompress(payload[1:], limit)` → `rlp.NewStream`, loop `Decode(&seg []byte)`
to EOF; each element = one segment. Zeroheavy (`:163`): decode `payload[1:]`, re-check flag.

### §3 — Segments (`arbstate/inbox.go:245`)
Kinds: **0 L2Message · 1 L2MessageBrotli · 2 DelayedMessages · 3 AdvanceTimestamp · 4 AdvanceL1BlockNumber**.
Framing: each segment is an RLP byte string inside the outer RLP list (RLP length prefix; no extra varint);
`segment[0]` = kind.
- Advance* (`:345`): `segment[1:]` = RLP-stream `Uint64()` **delta** (added to running counter).
- L2Message (`:405`): `segment[1:]` = raw L2msg bytes.
- L2MessageBrotli (`:394`): `segment[1:]` brotli, limit `MaxL2MessageSize = 262144`.
- DelayedMessages (`:419`): kind byte only → `ReadDelayedInbox(delayedMessagesRead)`.

### §4 — inboxMultiplexer Pop() (`arbstate/inbox.go:211,327`)
State: `cachedSegmentNum, timestamp, blockNumber, subMessageNumber, delayedMessagesRead`.
Walk from `cachedSegmentNum`: Advance segments mutate running ts/block; L2/Delayed segments below the target
submessage are skipped (both counters++). At the target: **clamp** `ts→[Min,Max]Timestamp`,
`block→[Min,Max]L1Block` (`:370`). If past end of segments, synthesize a virtual DelayedMessages segment (`:382`).
- L2Message/Brotli → `MessageWithMetadata{ Message: L1IncomingMessage{ Kind=3(L2Message), Poster=BatchPosterAddr,
  BlockNumber, Timestamp, RequestId=nil, L1BaseFee=0, L2msg }, DelayedMessagesRead }`.
- DelayedMessages → require `delayedMessagesRead < AfterDelayedMessages`; `ReadDelayedInbox(delayedMessagesRead)`;
  `delayedMessagesRead++`.

> Note: regular sequencer L2 messages carry **Poster = the chain's BatchPoster address** (a config constant),
> `L1BaseFee = 0`, `RequestId = nil`. Delayed messages carry their own header from the delayed inbox.

### §5 — L1IncomingMessage.Serialize() — 113-byte header (`arbos/arbostypes/incomingmessage.go:106`)
`[0]Kind(1) · [1..33]Poster(32, =12 zero bytes ++ 20-byte addr) · [33..41]BlockNumber(u64 BE) ·
[41..49]Timestamp(u64 BE) · [49..81]RequestId(32, err if nil) · [81..113]L1BaseFee(32, big-endian u256, err if nil) ·
[113..]L2msg(raw, no length prefix)`. Inverse: `ParseIncomingL1Message()` (`:250`).

### §6 — MessageWithMetadata + Hash (`arbos/arbostypes/messagewithmeta.go:13`)
`struct { Message *L1IncomingMessage; DelayedMessagesRead u64 }`. `Hash() = keccak256(rlp.EncodeToBytes(self))`.
`L1IncomingMessage` RLP fields: `Header, L2msg, LegacyBatchGasCost, BatchDataStats`. MEL accumulator
(`mel/state.go:125`): `newAcc = keccak256(prevAcc || keccak256(rlp(msg)))`.

### §7 — Delayed inbox (deferred to milestone 2)
`MessageDelivered` event (`IBridge.sol:33`): topics `[sig, messageIndex(uint256), beforeInboxAcc]`, data
`[inbox(addr), kind(u8), sender(addr), messageDataHash(b32), baseFeeL1(u256), timestamp(u64)]`. Raw bytes from
`InboxMessageDelivered`(event data) or `InboxMessageDeliveredFromOrigin`(calldata `sendL2MessageFromOrigin(bytes)`,
skip selector); verify `keccak256(data) == messageDataHash` (`delayed.go:208`).
On-chain accumulator (`Messages.sol:34`): `messageHash = keccak256(kind[1] ++ sender[20] ++ blockNumber[8] ++
timestamp[8] ++ inboxSeqNum[32] ++ baseFeeL1[32] ++ messageDataHash[32])`; `newAcc = keccak256(prevAcc ++ messageHash)`.

---

## Milestone 1 — scope, test vector, exit

**Scope:** calldata batch → `Vec<MessageWithMetadata>`. Sequencer L2 messages only (kinds 0/1/3/4); for a batch that
references delayed messages (kind 2), milestone 1 may stub `ReadDelayedInbox` (record the cursor) and validate the
sequencer-message subset. No blob, no DAS, no multi-batch sequencing, no live following.

**Test vector:** fetch ONE real mainnet `SequencerInbox` batch tx (calldata `dataLocation=0`), commit the raw batch
bytes as a fixture under `arb-reth-derive/tests/fixtures/`. Decode offline.

**Exit:** the decoder reproduces the batch's messages — validate by (a) message count matches the L2 block range that
batch advanced the chain by, and (b) each emitted `L2msg` parses to the same L2 transactions the chain executed
(cross-check against `arb_revm`'s existing L2-message/tx parsing, or compare `MessageWithMetadata.hash()` if canonical
hashes are obtainable). Pure unit test, no network.

**Deferred (milestone 2+):** delayed-inbox reconciliation (§7), blob batches (§1 blob path + 4844 sidecar),
zeroheavy/DAS, the live `l1source` adapter (SequencerInbox scan), multi-batch ordering → feeds Stage E.

## Open parameters (pending decision)
1. **L1 source for the fixture:** default **dRPC (free), fetched once** and committed. (Override if you have an L1 RPC.)
2. **Calldata-first target:** post-Dencun mainnet is mostly *blob* batches, so the calldata fixture may be a
   **pre-Dencun** batch (same format, older ArbOS) or an occasional recent calldata batch. Confirm calldata-first
   (recommended) vs. going straight to blobs.
3. **BatchPoster address constant** for mainnet (42161) — needed for the `Poster` field on emitted L2 messages.

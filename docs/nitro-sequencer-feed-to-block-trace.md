# Nitro STF Spec: Feed Ingestion -> Message Execution -> Block Append

This is the execution spec to wire `arb-revm` handlers against Nitro behavior.
Scope is the full non-sequencer node path from sequencer feed intake to canonical block append, including reorg branches and lock behavior.

## 0) Canonical input object

Feed messages are transformed into:

- `BroadcastFeedMessage`:
  - `SequenceNumber`
  - `Message` (`MessageWithMetadata`)
  - optional `BlockHash`
  - optional `BlockMetadata`
  - `Signature`
  - `nitro/broadcaster/message/message.go:43-51`
- `MessageWithMetadata`:
  - `Message` (`L1IncomingMessage`)
  - `DelayedMessagesRead`
  - `nitro/arbos/arbostypes/messagewithmeta.go:13-16`
- `L1IncomingMessageHeader`:
  - `Kind`, `Poster`, `BlockNumber`, `Timestamp`, `RequestId`, `L1BaseFee`
  - `nitro/arbos/arbostypes/incomingmessage.go:51-58`

Signature hash domain includes chain id and full execution envelope:
- seq num, block hash, block metadata, delayed cursor, header fields, raw `L2msg`.
- `nitro/broadcaster/message/message.go:62-90`

## 1) Feed receive and validation

Entry point: `BroadcastClient.runLoop`.
- `nitro/broadcastclient/broadcastclient.go:444-503`

Reads:
- websocket payload bytes
- local verifier config (`Verify.Dangerous.AcceptMissing`)
- chain id for signature domain

Conditionals:
- invalid JSON => drop message
- `Version != 1` => ignore payload
- nil feed message entry => skip entry
- signature verification failure in any entry => drop whole batch
- tx streamer hard-stop error => stop client loop

In-memory mutations:
- `connected` flip and connection gauges
- `nextSeqNum = message.SequenceNumber + 1` per validated entry

Durable writes:
- none in this layer

## 2) Consensus ingest (`TransactionStreamer.AddBroadcastMessages`)

Entry point: `AddBroadcastMessages`.
- `nitro/arbnode/transaction_streamer.go:639-741`

Reads:
- incoming feed array
- consensus DB (via duplicate detection)
- previous message existence when `broadcastFirstMsgIdx > 0`

Conditionals:
- empty batch => return
- sequence discontinuity => error
- nil `Message` or `Header` => error
- duplicate prefix present => trim duplicates
- `feedReorg` / queue state branches:
  - initialize queue
  - replace queue (older or jump case)
  - append to queue if contiguous
- active feed reorg => do not write yet
- missing previous message in DB => do not write yet

In-memory mutations:
- lock `insertionMutex` for all queue and dedup mutations
- mutate:
  - `broadcasterQueuedMessages`
  - `broadcasterQueuedMessagesFirstMsgIdx`
  - `broadcasterQueuedMessagesActiveReorg`

Durable writes:
- indirectly via `addMessagesAndEndBatchImpl(...)` when queue becomes committable

## 3) Duplicate detection rules (`countDuplicateMessages`)

Entry point: `countDuplicateMessages`.
- `nitro/arbnode/transaction_streamer.go:844-923`

Reads:
- `schema.MessagePrefix[msgIdx]` (existing RLP message)
- incoming candidate message RLP

Conditionals:
- DB key missing => stop duplicate scan
- byte-equal RLP => duplicate
- byte mismatch:
  - parse DB message
  - special-case tolerate mismatch if only batch gas cache differs
  - otherwise report reorg

In-memory mutations:
- none required for persistent behavior

Durable writes:
- optional: if mismatch is only cache fields and a batch is provided, overwrite message slot with richer cache payload (`writeMessage(...)` path)
- `nitro/arbnode/transaction_streamer.go:899-905`

## 4) Commit / reorg logic (`addMessagesAndEndBatchImpl`)

Entry point: `addMessagesAndEndBatchImpl`.
- `nitro/arbnode/transaction_streamer.go:944-1072`

Reads:
- queue head (`broadcasterQueuedMessagesFirstMsgIdx`)
- DB duplicate state
- previous delayed cursor (`getPrevPrevDelayedRead`)

Conditionals:
- confirmed input branch:
  - dedup with possible `confirmedReorg`
  - trim already persisted messages
- append feed cache to confirmed tail when compatible
- non-confirmed branch:
  - dedup and detect `feedReorg`
- if `feedReorg == true` => never reorg confirmed state, return without applying
- per-message delayed cursor validation:
  - `DelayedMessagesRead` delta must be `0` or `1`; otherwise reject
- if `confirmedReorg == true`:
  - run `addMessagesAndReorg(...)`
- no remaining messages => end batch

In-memory mutations:
- queue trimming/clearing after successful write
- reset `broadcasterQueuedMessagesActiveReorg` when queue consumed

Durable writes:
- reorg branch:
  - `addMessagesAndReorg` writes reorg effects
- normal branch:
  - `writeMessages(firstMsgIdx, messages, batch)`

## 5) Confirmed reorg path (`addMessagesAndReorg` + `ExecutionEngine.Reorg`)

Streamer side:
- `nitro/arbnode/transaction_streamer.go:311-467`

Execution side:
- `nitro/execution/gethexec/executionengine.go:421-491`

Reads:
- old messages from consensus DB to optionally resequence
- delayed accumulator / bridge lookups (if configured) for delayed-message resequence validity
- chain head blocks in execution engine

Conditionals:
- cannot reorg out init/genesis message
- max resequence depth cap
- delayed message resequence only if indices and accumulator checks match
- execution reorg target below safe/final blocks => clear safe/final markers

In-memory mutations:
- lock order:
  - streamer holds `insertionMutex`, then reorg path takes `reorgMutex`
  - execution takes `createBlocksMutex`
- optional resequencing handoff through `resequenceChan`

Durable writes:
- execution:
  - `bc.ReorgToOldBlock(...)`
  - digest and append replacement messages
- streamer DB cleanup from first reorged index:
  - delete `MessageResultPrefix`
  - delete `BlockHashInputFeedPrefix`
  - delete `BlockMetadataInputFeedPrefix`
  - delete `MissingBlockMetadataInputFeedPrefix`
  - delete `MessagePrefix`
  - reset `MessageCountKey`
- then store newly computed results for replacement segment

## 6) Consensus persistence shape (`writeMessage` / `writeMessages`)

Entry points:
- `writeMessage`: `nitro/arbnode/transaction_streamer.go:1247-1292`
- `writeMessages`: `nitro/arbnode/transaction_streamer.go:1320-1350`

Reads:
- existing block metadata when deciding missing-marker writes

Conditionals:
- `syncTillMessage` guard can halt block creation path
- metadata tracking only when `trackBlockMetadataFrom` gate is active
- missing metadata marker written only when metadata absent and previously absent

In-memory mutations:
- notifier channel send (`newMessageNotifier`) after successful DB write

Durable writes per message index:
- `schema.MessagePrefix` = RLP(`MessageWithMetadata`)
- `schema.BlockHashInputFeedPrefix` = optional feed hash
- optional `schema.BlockMetadataInputFeedPrefix`
- optional `schema.MissingBlockMetadataInputFeedPrefix`

Durable write at batch end:
- `schema.MessageCountKey` advanced to `first + len(messages)`

## 7) Execution scheduling (`Start` and `ExecuteNextMsg`)

Loop start:
- `Start` wires iterative executor on `newMessageNotifier`
- `nitro/arbnode/transaction_streamer.go:1610-1632`

Per-message execution:
- `ExecuteNextMsg`
- `nitro/arbnode/transaction_streamer.go:1454-1534`

Reads:
- `reorgMutex.TryRLock` gate
- consensus head msg index (`GetHeadMessageIndex`)
- execution head msg index (`exec.HeadMessageIndex`)
- current msg plus optional next msg for prefetch

Conditionals:
- context canceled => stop
- reorg lock unavailable => skip tick
- no consensus messages => idle
- execution already caught up or sync target reached => idle
- digest error => retry later

In-memory mutations:
- `prevHeadMsgIdx` updated for adaptive logging

Durable writes:
- after successful digest:
  - store `MessageResultPrefix[msgIdx]`

Non-durable outputs:
- call `checkResult` against feed-provided hash
- rebroadcast executed message with computed block hash

## 8) Block hash mismatch handling (`checkResult`)

Entry point:
- `nitro/arbnode/transaction_streamer.go:1406-1437`

Reads:
- feed-provided `BlockHash` and `BlockMetadata` for this message

Conditionals:
- no feed hash => no check
- mismatch => error log and optional fatal shutdown

In-memory mutations:
- none

Durable writes on mismatch (if tracked metadata exists):
- delete `BlockMetadataInputFeedPrefix[msgIdx]`
- write `MissingBlockMetadataInputFeedPrefix[msgIdx]`

## 9) Block creation lock and ordering (`DigestMessage`)

Entry points:
- `DigestMessage`: `nitro/execution/gethexec/executionengine.go:1124-1130`
- `digestMessageWithBlockMutex`: `...:1132-1167`

Reads:
- current execution head header
- expected next msg index derived from block number

Conditionals:
- `createBlocksMutex.TryLock` failure => reject with `"createBlock mutex held"`
- message index mismatch => reject
- optional prefetch goroutine if `prefetchBlock` and msg+1 exists

In-memory mutations:
- cache L1 price data window after append
- scheduled-upgrade check timer (`nextScheduledVersionCheck`)

Durable writes:
- block append via `appendBlock(...)`

## 10) Build block from message (`createBlockFromNextMessage`)

Entry point:
- `nitro/execution/gethexec/executionengine.go:852-965`

Reads:
- current canonical head and full block
- state trie root and recovered state
- message header + delayed cursor
- VM/stateless config flags

Conditionals:
- missing current block / recover state failure => error
- run context selection:
  - sequencing
  - prefetch
  - commit
- delayed sequencing filtering branch:
  - parse txs
  - run `DelayedFilteringSequencingHooks`
  - optional RPC report of filtered tx hashes
  - if filtered set non-empty => return `ErrFilteredDelayedMessage` (no append)
- otherwise use regular `arbos.ProduceBlock(...)`

In-memory mutations:
- address checker hook on `StateDB`
- prefetcher lifecycle start/stop

Durable writes:
- none in this function directly (write occurs in `appendBlock`)

## 11) STF core mutation path (`arbos.ProduceBlockAdvanced`)

Entry point:
- `nitro/arbos/block_processor.go:294-698`

Reads:
- system ArbOS state from `StateDB` (`OpenSystemArbosState`)
- incoming L1 header fields (`Poster`, `BlockNumber`, `Timestamp`, `L1BaseFee`)
- ArbOS pricing state (`CommitMultiGasFees`, `BaseFeeWei`, `PerBlockGasLimit`)
- previous header for monotonic timestamp and parent hash

Conditionals and mutation mechanics:
- reject dirty `StateDB` unexpected balance delta before start
- create header:
  - `createNewHeader(...)` sets `Time=max(prev.Time,l1Timestamp)`, `Coinbase=poster`, parent linkage
  - `nitro/arbos/block_processor.go:164-203`
- prepend `InternalTxStartBlock(...)` as first tx
- tx loop branches:
  - first internal tx
  - scheduled redeem tx FIFO
  - next user tx from hooks
- pre filters:
  - hooks pre-filter
  - extra pre-filter
- transaction apply:
  - snapshot -> execute -> revert on failure
  - post-filter and optional rollback checkpoints for grouped redeems
- failure handling:
  - user tx can be discarded depending on hooks
  - otherwise deduct block-local gas budget
- internal tx success:
  - reopen ArbOS state and refresh header ArbOS version if upgraded
- gas/accounting updates:
  - poster gas accounting
  - expected balance delta tracking from deposits/withdrawals
  - block-local gas budget decrement
- append successful tx/receipt and process scheduled redeems
- final block filter hook

Key internal start-block state mutations:
- `ApplyInternalTxUpdate` on `InternalTxStartBlock`:
  - update L1 blockhash progression (`RecordNewL1Block`)
  - reap retryables (two attempts)
  - update L2 pricing model
  - trigger ArbOS upgrade if scheduled
- `nitro/arbos/internal_tx.go:68-110`

Finalize:
- `header.Nonce = delayedMessagesRead`
- `FinalizeBlock(...)` computes:
  - `SendRoot`, `SendCount`, `L1BlockNumber`, `ArbOSFormatVersion`, `CollectTips`
  - header root via `statedb.IntermediateRoot(true)`
- `nitro/arbos/block_processor.go:666-747`

Header packing format:
- `MixDigest[0:8]` = `SendCount`
- `MixDigest[8:16]` = `L1BlockNumber`
- `MixDigest[16:24]` = `ArbOSFormatVersion`
- `MixDigest[25]` = `CollectTips` bit
- `Extra` = `SendRoot`
- `nitro/go-ethereum/core/types/arb_types.go:623-671`

Durable writes:
- none yet; still in-memory `StateDB` + constructed block object

## 12) Canonical append (`appendBlock`)

Entry point:
- `nitro/execution/gethexec/executionengine.go:968-1013`

Reads:
- produced block, receipts, state transition outputs
- tracer mode config

Conditionals:
- tracer enabled => `InsertChain` path
- normal => `WriteBlockAndSetHeadWithTime`
- reject non-canonical side status

In-memory mutations:
- metrics/gauges and L1 gas estimate metric refresh

Durable writes:
- canonical block, receipts, logs, and head update in execution DB

## 13) Complete non-sequencer call graph (linear view)

1. `BroadcastClient.runLoop` validates feed envelope/signatures.
2. `TransactionStreamer.AddBroadcastMessages` normalizes feed -> queue.
3. `countDuplicateMessages` trims duplicates and detects reorg.
4. `addMessagesAndEndBatchImpl` validates delayed cursor and commits messages.
5. `writeMessages` persists consensus message rows and advances message count.
6. notifier wakes `ExecuteNextMsg`.
7. `ExecuteNextMsg` loads message N (+ optional N+1 prefetch).
8. `ExecutionEngine.DigestMessage` takes `createBlocksMutex`.
9. `createBlockFromNextMessage` opens state and invokes ArbOS STF.
10. `arbos.ProduceBlockAdvanced` mutates state and finalizes header info.
11. `appendBlock` writes canonical block.
12. `ExecuteNextMsg` stores `MessageResultPrefix[N]` and rebroadcasts with computed block hash.

## 14) Handler wiring implications for `arb-revm`

For parity-focused handler wiring, the execution boundary must accept exactly:

- `MessageWithMetadata` as authoritative pre-block input.
- Parent block header/state root from DB.
- An execution mode (`commit` vs `prefetch` vs `sequencing`) equivalent.
- A lock discipline equivalent to `createBlocksMutex` for block construction.

State that must not be hidden in long-lived executor memory:

- no cross-tx retained EVM state
- no hidden delayed cursor cache
- no derived header cache treated as source of truth

Only durable authority:

- consensus/message DB rows for message stream and message results
- execution DB canonical chain state

This is the spec to wire handlers first; value-by-value context slimming (what belongs in `ArbChainContext` vs per-message input) should be derived from this flow, not inferred ad hoc.

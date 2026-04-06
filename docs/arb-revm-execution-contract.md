# arb-revm Execution Contract (Step 1)

This document freezes the current execution boundary before handler wiring.

## Input contract

- `ArbExecutionInput`
  - `parent: ArbParentHeader`
  - `message: ArbMessageEnvelope`
  - `cfg: ArbExecCfg`
  - `mode: ArbExecutionMode`

No hidden executor-local state is read. Everything execution-critical must be in the input or DB.

## Output contract

- `ArbExecOutcome`
  - execution counters (`attempted`, `executed`, `skipped_unsupported`)
  - `start_block_success`, `start_block_gas_used`
  - per-tx summaries (`ArbTxExecution`)
  - explicit durable side effects (`writes: Vec<ArbWriteEffect>`)

## Durable write intent model

- `ArbWriteTarget::StateDatabase` is currently the only write target.
- `ArbWriteStage` identifies when a state write happened:
  - `StartBlockPrelude`
  - `UserTransaction`

Current mapping:

1. `StartBlockPrelude` write is emitted after the start-block prelude in commit-capable modes.
2. `UserTransaction` write is emitted once per successfully executed + committed user tx.

## Mode policy

- `ArbExecutionMode::Commit`:
  - executes and commits state.
  - emits `writes`.
- `ArbExecutionMode::Prefetch`:
  - executes without persisting state.
  - emits no `writes`.
- `ArbExecutionMode::Sequencing`:
  - currently same state-commit behavior as `Commit`.
  - kept separate as a stable API seam for sequencer-specific logic.

## Hook extension points

`execute_message_with_hooks` accepts `ArbExecutionHooks`:

- `start_block_prelude(input, derived)` -> `Option<ArbSystemCall>`
  - `Some(...)`: execute configured prelude as typed internal tx (`0x6a`).
  - `None`: skip prelude.

Default behavior is `DefaultArbExecutionHooks`, which encodes ArbOSActs
`startBlock(...)` as the prelude call.

## Runner lock model

- `ArbRunner` wraps execution with `try_lock` serialization.
- if lock acquisition fails: `ArbRunnerError::LockHeld`.
- if lock succeeds: execution proceeds through `execute_message_with_hooks`.

This mirrors Nitro's fast-fail lock behavior and keeps lock policy outside the EVM core.

Consensus DB writes are out of scope for this function and must be handled by a higher layer.

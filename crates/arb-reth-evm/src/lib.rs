//! `arb-reth-evm` — bridge `arb_revm` (Arbitrum/ArbOS execution on revm 36) into reth's
//! EVM and block-execution extension points.
//!
//! Phase 1 of the arb-reth roadmap (see `docs/arb-reth-roadmap.md`). This crate is currently
//! a scaffold whose only job is to prove the dependency graph unifies: `arb_revm` and reth
//! v2.0.0 must agree on a single revm 36. The real `ConfigureEvm` / `BlockExecutor` impls
//! land here next.

// Force both halves of the graph into the build so version unification is actually exercised.
use arb_revm as _;
use reth_evm as _;
// Arbitrum primitives (tx envelope, receipt, header info) — forces arb-alloy to compile against
// reth's unified alloy 1.8.3, proving no breaking API drift from its pinned 1.6.3.
use arb_alloy_consensus as _;

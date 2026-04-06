/// Arbitrum chain-scoped execution context carried alongside block/tx/cfg.
///
/// This must stay minimal and should not duplicate values already present in
/// block env or transaction/message env.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ArbChainContext {
    /// Sequencer feed sequence number for this message.
    pub sequence_number: Option<u64>,
}

impl ArbChainContext {
    /// Creates a lean chain context.
    pub fn new(sequence_number: Option<u64>) -> Self {
        Self { sequence_number }
    }

    /// Sets the sequence number.
    pub fn with_sequence_number(mut self, sequence_number: Option<u64>) -> Self {
        self.sequence_number = sequence_number;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::ArbChainContext;

    #[test]
    fn builds_chain_context_from_non_block_inputs() {
        let ctx = ArbChainContext::new(Some(42));
        assert_eq!(ctx.sequence_number, Some(42));
    }
}

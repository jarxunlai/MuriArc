use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use crate::ImportError;

/// Cooperative cancellation hook used by parsing, preview, and export loops.
///
/// Callers may implement this trait with their own job cancellation state. A
/// cancellation check never performs cleanup itself: importer operations only
/// build in-memory previews or write to caller-owned output streams.
pub trait CancellationCheck: Send + Sync {
    fn is_cancelled(&self) -> bool;

    fn check_cancelled(&self) -> Result<(), ImportError> {
        if self.is_cancelled() {
            Err(ImportError::Cancelled)
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NoCancellation;

impl CancellationCheck for NoCancellation {
    fn is_cancelled(&self) -> bool {
        false
    }
}

#[derive(Debug, Clone, Default)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

impl CancellationCheck for CancellationToken {
    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_is_shareable_and_monotonic() {
        let token = CancellationToken::default();
        let other = token.clone();
        assert!(!token.is_cancelled());
        other.cancel();
        assert!(token.is_cancelled());
        assert!(matches!(
            token.check_cancelled(),
            Err(ImportError::Cancelled)
        ));
    }
}

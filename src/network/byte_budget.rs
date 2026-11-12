//! Byte budgets for transport queues (TODO 8.5.2 finding F6).
//!
//! Queues bounded only by message count can hold count × 16 MiB frames. A `ByteBudget` bounds
//! the bytes a queue may hold. A frame takes a [`Reservation`] before it is buffered; the
//! reservation travels with the queued item and returns its bytes when dropped, so no code path
//! (error, full queue, shutdown, panic) can leak budget.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Debug)]
pub(crate) struct ByteBudget {
    used: AtomicUsize,
    limit: usize,
}

impl ByteBudget {
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            used: AtomicUsize::new(0),
            limit,
        }
    }

    fn try_take(&self, bytes: usize) -> bool {
        self.used
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(bytes).filter(|total| *total <= self.limit)
            })
            .is_ok()
    }

    fn give_back(&self, bytes: usize) {
        let _ = self
            .used
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                Some(used.saturating_sub(bytes))
            });
    }

    #[cfg(test)]
    pub(crate) fn used(&self) -> usize {
        self.used.load(Ordering::Acquire)
    }
}

/// Bytes held in one or more budgets; returned on drop.
#[derive(Debug)]
#[must_use = "dropping a reservation returns its bytes at once"]
pub(crate) struct Reservation {
    budgets: Vec<Arc<ByteBudget>>,
    bytes: usize,
}

impl Reservation {
    /// A reservation that holds nothing (local delivery).
    pub(crate) fn none() -> Self {
        Self {
            budgets: Vec::new(),
            bytes: 0,
        }
    }

    /// Take `bytes` from every budget, or from none if any of them is full.
    pub(crate) fn try_new(budgets: &[&Arc<ByteBudget>], bytes: usize) -> Option<Self> {
        let mut taken = Self {
            budgets: Vec::with_capacity(budgets.len()),
            bytes,
        };
        for budget in budgets {
            if !budget.try_take(bytes) {
                return None; // `taken` drops here and returns what it already holds.
            }
            taken.budgets.push(Arc::clone(budget));
        }
        Some(taken)
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        for budget in &self.budgets {
            budget.give_back(self.bytes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reservations_stay_within_the_limit_and_return_on_drop() {
        let budget = Arc::new(ByteBudget::new(100));
        let first = Reservation::try_new(&[&budget], 60).expect("fits");
        assert!(
            Reservation::try_new(&[&budget], 41).is_none(),
            "over the limit"
        );
        let second = Reservation::try_new(&[&budget], 40).expect("exactly the limit");
        assert!(Reservation::try_new(&[&budget], 1).is_none());
        drop(first);
        assert_eq!(budget.used(), 40);
        drop(second);
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn test_failed_multi_budget_reservation_takes_nothing() {
        let peer = Arc::new(ByteBudget::new(100));
        let global = Arc::new(ByteBudget::new(50));
        assert!(Reservation::try_new(&[&peer, &global], 60).is_none());
        assert_eq!((peer.used(), global.used()), (0, 0));
        let held = Reservation::try_new(&[&peer, &global], 50).expect("fits both");
        assert_eq!((peer.used(), global.used()), (50, 50));
        drop(held);
        assert_eq!((peer.used(), global.used()), (0, 0));
    }

    #[test]
    fn test_overflowing_reservation_is_refused() {
        let budget = Arc::new(ByteBudget::new(usize::MAX));
        let all = Reservation::try_new(&[&budget], usize::MAX).expect("fits");
        assert!(Reservation::try_new(&[&budget], 1).is_none());
        drop(all);
        assert_eq!(budget.used(), 0);
        drop(Reservation::none());
    }
}

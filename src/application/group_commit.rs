use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, Sender};

use super::{MAX_GROUP_TRANSACTIONS, WriteBatch};
use crate::{Error, Result, TransactionId};

pub(super) struct BatchCommitCoordinator {
    state: Mutex<QueueState>,
}

impl BatchCommitCoordinator {
    pub(super) const fn new() -> Self {
        Self {
            state: Mutex::new(QueueState {
                leader_active: false,
                failed: false,
                pending: VecDeque::new(),
            }),
        }
    }

    pub(super) fn enqueue(&self, batch: WriteBatch) -> WaitingBatch {
        let (completion, result) = mpsc::channel();
        let mut state = self.state();
        if state.failed {
            let _ = completion.send(Completion::Complete(Err(Error::LockPoisoned(
                "automatic group commit",
            ))));
            return WaitingBatch {
                result,
                is_leader: false,
            };
        }
        if state.pending.try_reserve(1).is_err() {
            let _ = completion.send(Completion::Complete(Err(Error::FileTooLarge)));
            return WaitingBatch {
                result,
                is_leader: false,
            };
        }
        state.pending.push_back(PendingBatch { batch, completion });
        let is_leader = !state.leader_active;
        if is_leader {
            state.leader_active = true;
        }
        WaitingBatch { result, is_leader }
    }

    pub(super) fn take_group(&self) -> Result<Vec<PendingBatch>> {
        self.take_group_up_to(MAX_GROUP_TRANSACTIONS)
    }

    fn take_group_up_to(&self, limit: usize) -> Result<Vec<PendingBatch>> {
        let mut state = self.state();
        let count = state.pending.len().min(limit);
        let mut group = Vec::new();
        group
            .try_reserve_exact(count)
            .map_err(|_| Error::FileTooLarge)?;
        for _ in 0..count {
            let request = state.pending.pop_front().ok_or(Error::InternalInvariant(
                "automatic group pending queue changed",
            ))?;
            group.push(request);
        }
        Ok(group)
    }

    pub(super) fn reject_pending(&self, error: &Error) {
        let pending = {
            let mut state = self.state();
            std::mem::take(&mut state.pending)
        };
        complete_error(pending, error);
    }

    pub(super) fn leadership_guard(&self) -> LeadershipGuard<'_> {
        LeadershipGuard { coordinator: self }
    }

    fn handoff(&self) {
        let mut state = self.state();
        if state.failed {
            return;
        }
        while let Some(request) = state.pending.front() {
            if request.completion.send(Completion::Lead).is_ok() {
                return;
            }
            state.pending.pop_front();
        }
        state.leader_active = false;
    }

    pub(super) fn fail_closed(&self) {
        let pending = {
            let mut state = self.state();
            state.failed = true;
            state.leader_active = false;
            std::mem::take(&mut state.pending)
        };
        complete_error(pending, &Error::LockPoisoned("automatic group commit"));
    }

    pub(super) fn reset_after_refresh(&self) {
        let mut state = self.state();
        if state.failed && !state.leader_active && state.pending.is_empty() {
            state.failed = false;
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, QueueState> {
        match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

pub(super) struct LeadershipGuard<'coordinator> {
    coordinator: &'coordinator BatchCommitCoordinator,
}

impl Drop for LeadershipGuard<'_> {
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.coordinator.fail_closed();
        } else {
            self.coordinator.handoff();
        }
    }
}

struct QueueState {
    leader_active: bool,
    failed: bool,
    pending: VecDeque<PendingBatch>,
}

pub(super) struct PendingBatch {
    batch: WriteBatch,
    completion: Sender<Completion>,
}

impl PendingBatch {
    pub(super) fn take_batch(&mut self) -> WriteBatch {
        std::mem::take(&mut self.batch)
    }
}

pub(super) fn complete_group(pending: Vec<PendingBatch>, result: Result<Vec<TransactionId>>) {
    match result {
        Ok(transaction_ids) if transaction_ids.len() == pending.len() => {
            for (request, transaction_id) in pending.into_iter().zip(transaction_ids) {
                let _ = request
                    .completion
                    .send(Completion::Complete(Ok(transaction_id)));
            }
        }
        Ok(_) => complete_error(
            pending,
            &Error::InternalInvariant("automatic group result length"),
        ),
        Err(error) => complete_error(pending, &error),
    }
}

fn complete_error(pending: impl IntoIterator<Item = PendingBatch>, error: &Error) {
    for request in pending {
        let _ = request
            .completion
            .send(Completion::Complete(Err(error.duplicate())));
    }
}

enum Completion {
    Complete(Result<TransactionId>),
    Lead,
}

pub(super) struct WaitingBatch {
    result: Receiver<Completion>,
    pub(super) is_leader: bool,
}

impl WaitingBatch {
    pub(super) fn receive(&self) -> WaitAction {
        self.result.recv().map_or_else(
            |_| WaitAction::Complete(Err(Error::LockPoisoned("automatic group commit"))),
            |completion| match completion {
                Completion::Complete(result) => WaitAction::Complete(result),
                Completion::Lead => WaitAction::Lead,
            },
        )
    }
}

pub(super) enum WaitAction {
    Complete(Result<TransactionId>),
    Lead,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_leader_drains_batches_in_queue_order() {
        let coordinator = BatchCommitCoordinator::new();
        let first = coordinator.enqueue(WriteBatch::new());
        let second = coordinator.enqueue(WriteBatch::new());
        assert!(first.is_leader);
        assert!(!second.is_leader);

        let group = coordinator.take_group().unwrap();
        complete_group(
            group,
            Ok(vec![TransactionId::from_raw(4), TransactionId::from_raw(5)]),
        );
        assert!(matches!(
            first.receive(),
            WaitAction::Complete(Ok(transaction)) if transaction.value() == 4
        ));
        assert!(matches!(
            second.receive(),
            WaitAction::Complete(Ok(transaction)) if transaction.value() == 5
        ));
        coordinator.handoff();

        let next = coordinator.enqueue(WriteBatch::new());
        assert!(next.is_leader);
    }

    #[test]
    fn a_group_error_reaches_every_waiter() {
        let coordinator = BatchCommitCoordinator::new();
        let first = coordinator.enqueue(WriteBatch::new());
        let second = coordinator.enqueue(WriteBatch::new());
        complete_group(coordinator.take_group().unwrap(), Err(Error::WriteConflict));
        assert!(matches!(
            first.receive(),
            WaitAction::Complete(Err(Error::WriteConflict))
        ));
        assert!(matches!(
            second.receive(),
            WaitAction::Complete(Err(Error::WriteConflict))
        ));
        coordinator.handoff();
    }

    #[test]
    fn leadership_is_handed_to_a_request_in_the_next_group() {
        let coordinator = BatchCommitCoordinator::new();
        let first = coordinator.enqueue(WriteBatch::new());
        let first_group = coordinator.take_group().unwrap();
        let second = coordinator.enqueue(WriteBatch::new());
        assert!(first.is_leader);
        assert!(!second.is_leader);

        complete_group(first_group, Ok(vec![TransactionId::from_raw(1)]));
        coordinator.handoff();
        assert!(matches!(second.receive(), WaitAction::Lead));
    }

    #[test]
    fn bounded_groups_preserve_the_pending_queue_order() {
        let coordinator = BatchCommitCoordinator::new();
        let first = coordinator.enqueue(WriteBatch::new());
        let second = coordinator.enqueue(WriteBatch::new());
        let third = coordinator.enqueue(WriteBatch::new());

        let group = coordinator.take_group_up_to(2).unwrap();
        complete_group(
            group,
            Ok(vec![TransactionId::from_raw(1), TransactionId::from_raw(2)]),
        );
        assert!(matches!(
            first.receive(),
            WaitAction::Complete(Ok(transaction)) if transaction.value() == 1
        ));
        assert!(matches!(
            second.receive(),
            WaitAction::Complete(Ok(transaction)) if transaction.value() == 2
        ));

        coordinator.handoff();
        assert!(matches!(third.receive(), WaitAction::Lead));
        assert_eq!(coordinator.take_group().unwrap().len(), 1);
    }

    #[test]
    fn handoff_skips_many_disconnected_waiters_in_queue_order() {
        const DISCONNECTED: usize = 4_096;

        let coordinator = BatchCommitCoordinator::new();
        let first = coordinator.enqueue(WriteBatch::new());
        complete_group(
            coordinator.take_group().unwrap(),
            Ok(vec![TransactionId::from_raw(1)]),
        );
        assert!(matches!(
            first.receive(),
            WaitAction::Complete(Ok(transaction)) if transaction.value() == 1
        ));

        for _ in 0..DISCONNECTED {
            drop(coordinator.enqueue(WriteBatch::new()));
        }
        let connected = coordinator.enqueue(WriteBatch::new());
        coordinator.handoff();

        assert!(matches!(connected.receive(), WaitAction::Lead));
        assert_eq!(coordinator.take_group().unwrap().len(), 1);
    }

    #[test]
    fn coordinator_fails_closed_when_a_leader_unwinds() {
        let coordinator = BatchCommitCoordinator::new();
        let first = coordinator.enqueue(WriteBatch::new());
        let first_group = coordinator.take_group().unwrap();
        let second = coordinator.enqueue(WriteBatch::new());
        assert!(first.is_leader);
        assert!(!second.is_leader);

        let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = coordinator.leadership_guard();
            let _owned_group = first_group;
            panic!("simulated automatic group leader failure");
        }));

        assert!(unwind.is_err());
        assert!(matches!(
            first.receive(),
            WaitAction::Complete(Err(Error::LockPoisoned("automatic group commit")))
        ));
        assert!(matches!(
            second.receive(),
            WaitAction::Complete(Err(Error::LockPoisoned("automatic group commit")))
        ));
        let future = coordinator.enqueue(WriteBatch::new());
        assert!(!future.is_leader);
        assert!(matches!(
            future.receive(),
            WaitAction::Complete(Err(Error::LockPoisoned("automatic group commit")))
        ));

        coordinator.reset_after_refresh();
        let recovered = coordinator.enqueue(WriteBatch::new());
        assert!(recovered.is_leader);
    }

    #[test]
    fn staging_failure_rejects_waiters_without_poisoning_future_groups() {
        let coordinator = BatchCommitCoordinator::new();
        let first = coordinator.enqueue(WriteBatch::new());
        let second = coordinator.enqueue(WriteBatch::new());

        let queued_during_rejection = {
            let _leadership = coordinator.leadership_guard();
            coordinator.reject_pending(&Error::FileTooLarge);
            let queued = coordinator.enqueue(WriteBatch::new());
            assert!(!queued.is_leader);
            queued
        };

        assert!(matches!(
            first.receive(),
            WaitAction::Complete(Err(Error::FileTooLarge))
        ));
        assert!(matches!(
            second.receive(),
            WaitAction::Complete(Err(Error::FileTooLarge))
        ));
        assert!(matches!(
            queued_during_rejection.receive(),
            WaitAction::Lead
        ));
        {
            let _leadership = coordinator.leadership_guard();
            complete_group(
                coordinator.take_group().unwrap(),
                Ok(vec![TransactionId::from_raw(1)]),
            );
        }
        assert!(matches!(
            queued_during_rejection.receive(),
            WaitAction::Complete(Ok(transaction)) if transaction.value() == 1
        ));
        let future = coordinator.enqueue(WriteBatch::new());
        assert!(future.is_leader);
    }
}

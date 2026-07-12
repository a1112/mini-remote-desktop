//! Fail-closed, in-memory trusted desktop state publication.

use crate::runtime::{TrustedDesktopState, TrustedDesktopStateSource};
use mrd_agent_ipc::DesktopKind;
use std::sync::{Arc, RwLock};
use tokio::sync::watch;

/// Failure to publish a trusted desktop transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DesktopPublishError {
    /// The monotonic desktop epoch cannot be advanced without wrapping.
    EpochOverflow,
    /// The read side has gone away or this publisher was already failed closed.
    SourceClosed,
}

/// The sole writer for one cached trusted desktop source.
///
/// This type is intentionally not cloneable. Dropping it clears the cached
/// snapshot before closing every subscription.
pub(crate) struct TrustedDesktopPublisher {
    snapshot: Arc<RwLock<Option<TrustedDesktopState>>>,
    sender: Option<watch::Sender<()>>,
}

impl TrustedDesktopPublisher {
    /// Publish one trusted native transition, including same-kind transitions.
    ///
    /// The complete snapshot is stored before subscribers are notified. Epoch
    /// exhaustion permanently fails this publisher closed.
    pub(crate) fn publish_transition(
        &mut self,
        desktop_kind: DesktopKind,
    ) -> Result<TrustedDesktopState, DesktopPublishError> {
        self.publish_transition_inner(desktop_kind, |_| {})
    }

    #[cfg(test)]
    fn publish_transition_before_notify(
        &mut self,
        desktop_kind: DesktopKind,
        before_notify: impl FnOnce(TrustedDesktopState),
    ) -> Result<TrustedDesktopState, DesktopPublishError> {
        self.publish_transition_inner(desktop_kind, before_notify)
    }

    fn publish_transition_inner(
        &mut self,
        desktop_kind: DesktopKind,
        before_notify: impl FnOnce(TrustedDesktopState),
    ) -> Result<TrustedDesktopState, DesktopPublishError> {
        if self.sender.is_none() {
            self.fail_closed();
            return Err(DesktopPublishError::SourceClosed);
        }

        let next = {
            let mut snapshot = match self.snapshot.write() {
                Ok(snapshot) => snapshot,
                Err(poisoned) => {
                    let mut snapshot = poisoned.into_inner();
                    *snapshot = None;
                    drop(snapshot);
                    self.sender.take();
                    return Err(DesktopPublishError::SourceClosed);
                }
            };
            let Some(current) = *snapshot else {
                drop(snapshot);
                self.fail_closed();
                return Err(DesktopPublishError::SourceClosed);
            };
            let Some(desktop_epoch) = current.desktop_epoch.checked_add(1) else {
                *snapshot = None;
                drop(snapshot);
                self.sender.take();
                return Err(DesktopPublishError::EpochOverflow);
            };
            let next = TrustedDesktopState {
                desktop_epoch,
                desktop_kind,
            };
            *snapshot = Some(next);
            next
        };

        before_notify(next);
        if self
            .sender
            .as_ref()
            .is_none_or(|sender| sender.send(()).is_err())
        {
            self.fail_closed();
            return Err(DesktopPublishError::SourceClosed);
        }

        Ok(next)
    }

    /// Clear the snapshot and close all subscriptions.
    pub(crate) fn fail_closed(&mut self) {
        self.fail_closed_inner(|| {});
    }

    #[cfg(test)]
    fn fail_closed_before_close(&mut self, before_close: impl FnOnce()) {
        self.fail_closed_inner(before_close);
    }

    fn fail_closed_inner(&mut self, before_close: impl FnOnce()) {
        let mut snapshot = self
            .snapshot
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *snapshot = None;
        drop(snapshot);
        before_close();
        self.sender.take();
    }
}

impl Drop for TrustedDesktopPublisher {
    fn drop(&mut self) {
        self.fail_closed();
    }
}

/// Read-only in-memory view of the latest trusted desktop observation.
pub(crate) struct CachedDesktopStateSource {
    snapshot: Arc<RwLock<Option<TrustedDesktopState>>>,
    receiver: watch::Receiver<()>,
}

impl TrustedDesktopStateSource for CachedDesktopStateSource {
    fn current_state(&self) -> Option<TrustedDesktopState> {
        self.snapshot.read().ok().and_then(|snapshot| *snapshot)
    }

    fn subscribe(&self) -> watch::Receiver<()> {
        let mut receiver = self.receiver.clone();
        receiver.borrow_and_update();
        receiver
    }
}

/// Create a trusted desktop cache with epoch one and an initial observation.
///
/// The returned publisher is the only owner of the notification sender. The
/// source remains entirely read-only and never performs platform I/O.
pub(crate) fn trusted_desktop_cache(
    initial_kind: DesktopKind,
) -> (TrustedDesktopPublisher, CachedDesktopStateSource) {
    desktop_state_cache(1, initial_kind)
}

fn desktop_state_cache(
    initial_epoch: u64,
    initial_kind: DesktopKind,
) -> (TrustedDesktopPublisher, CachedDesktopStateSource) {
    let snapshot = Arc::new(RwLock::new(Some(TrustedDesktopState {
        desktop_epoch: initial_epoch,
        desktop_kind: initial_kind,
    })));
    let (sender, receiver) = watch::channel(());
    (
        TrustedDesktopPublisher {
            snapshot: Arc::clone(&snapshot),
            sender: Some(sender),
        },
        CachedDesktopStateSource { snapshot, receiver },
    )
}

#[cfg(test)]
mod tests {
    use super::{
        desktop_state_cache, trusted_desktop_cache, DesktopPublishError, TrustedDesktopPublisher,
    };
    use crate::runtime::{TrustedDesktopState, TrustedDesktopStateSource};
    use mrd_agent_ipc::DesktopKind;
    use std::sync::Arc;

    #[test]
    fn initial_snapshot_has_a_nonzero_epoch() {
        let (_publisher, source) = trusted_desktop_cache(DesktopKind::Default);

        assert_eq!(
            source.current_state(),
            Some(TrustedDesktopState {
                desktop_epoch: 1,
                desktop_kind: DesktopKind::Default,
            })
        );
    }

    #[test]
    fn transition_updates_snapshot_before_notifying_subscribers() {
        let (mut publisher, source) = trusted_desktop_cache(DesktopKind::Default);
        let receiver = source.subscribe();

        let published = publisher
            .publish_transition_before_notify(DesktopKind::Secure, |next| {
                assert_eq!(source.current_state(), Some(next));
                assert!(
                    !receiver
                        .has_changed()
                        .expect("publisher should remain available"),
                    "subscribers must not wake before the complete snapshot is visible",
                );
            })
            .expect("transition should publish");

        assert!(receiver
            .has_changed()
            .expect("publisher should remain available"));
        assert_eq!(source.current_state(), Some(published));
    }

    #[tokio::test]
    async fn coalesced_notifications_read_the_latest_snapshot() {
        let (mut publisher, source) = trusted_desktop_cache(DesktopKind::Default);
        let mut receiver = source.subscribe();

        publisher
            .publish_transition(DesktopKind::Secure)
            .expect("first transition should publish");
        publisher
            .publish_transition(DesktopKind::Winlogon)
            .expect("second transition should publish");

        receiver
            .changed()
            .await
            .expect("publisher should remain available");
        assert_eq!(
            source.current_state(),
            Some(TrustedDesktopState {
                desktop_epoch: 3,
                desktop_kind: DesktopKind::Winlogon,
            })
        );
    }

    #[test]
    fn same_kind_trusted_transition_still_advances_epoch() {
        let (mut publisher, source) = trusted_desktop_cache(DesktopKind::Default);
        let receiver = source.subscribe();

        let published = publisher
            .publish_transition(DesktopKind::Default)
            .expect("same-kind transition should publish");

        assert_eq!(published.desktop_epoch, 2);
        assert_eq!(published.desktop_kind, DesktopKind::Default);
        assert!(receiver
            .has_changed()
            .expect("publisher should remain available"));
        assert_eq!(source.current_state(), Some(published));
    }

    #[test]
    fn default_nondefault_default_aba_has_distinct_epochs() {
        let (mut publisher, source) = trusted_desktop_cache(DesktopKind::Default);

        let nondefault = publisher
            .publish_transition(DesktopKind::Winlogon)
            .expect("non-default transition should publish");
        let returned = publisher
            .publish_transition(DesktopKind::Default)
            .expect("return transition should publish");

        assert_eq!(nondefault.desktop_epoch, 2);
        assert_eq!(returned.desktop_epoch, 3);
        assert_eq!(returned.desktop_kind, DesktopKind::Default);
        assert_eq!(source.current_state(), Some(returned));
    }

    #[tokio::test]
    async fn epoch_overflow_clears_snapshot_and_closes_subscribers() {
        let (mut publisher, source) = desktop_state_cache(u64::MAX, DesktopKind::Default);
        let mut receiver = source.subscribe();

        assert_eq!(
            publisher.publish_transition(DesktopKind::Secure),
            Err(DesktopPublishError::EpochOverflow)
        );
        assert_eq!(source.current_state(), None);
        assert!(receiver.changed().await.is_err());
    }

    #[tokio::test]
    async fn poisoned_snapshot_fails_the_publisher_closed_without_revival() {
        let (mut publisher, source) = trusted_desktop_cache(DesktopKind::Default);
        let mut receiver = source.subscribe();
        let snapshot = Arc::clone(&publisher.snapshot);
        let _ = std::panic::catch_unwind(move || {
            let _guard = snapshot.write().expect("snapshot lock");
            panic!("poison the snapshot lock");
        });

        assert_eq!(
            publisher.publish_transition(DesktopKind::Secure),
            Err(DesktopPublishError::SourceClosed),
        );
        assert_eq!(source.current_state(), None);
        assert!(receiver.changed().await.is_err());
        assert_eq!(
            publisher.publish_transition(DesktopKind::Default),
            Err(DesktopPublishError::SourceClosed),
        );
    }

    #[tokio::test]
    async fn losing_the_only_publisher_clears_snapshot_and_closes_subscribers() {
        let (publisher, source) = trusted_desktop_cache(DesktopKind::Default);
        let mut receiver = source.subscribe();

        drop(publisher);

        assert_eq!(source.current_state(), None);
        assert!(receiver.changed().await.is_err());
    }

    #[tokio::test]
    async fn explicit_failure_clears_snapshot_before_closing_subscribers() {
        let (mut publisher, source) = trusted_desktop_cache(DesktopKind::Default);
        let mut receiver = source.subscribe();

        publisher.fail_closed_before_close(|| {
            assert_eq!(source.current_state(), None);
            assert!(!receiver
                .has_changed()
                .expect("publisher should still own the sender before close"));
        });

        assert!(receiver.changed().await.is_err());
    }

    #[test]
    fn source_keeps_subscriptions_open_while_publisher_is_alive() {
        let (_publisher, source) = trusted_desktop_cache(DesktopKind::Default);
        let receiver = source.subscribe();

        assert!(!receiver
            .has_changed()
            .expect("publisher should keep subscriptions open"));
    }

    #[test]
    fn late_subscription_starts_at_the_current_revision() {
        let (mut publisher, source) = trusted_desktop_cache(DesktopKind::Default);
        publisher
            .publish_transition(DesktopKind::Secure)
            .expect("transition should publish");

        let receiver = source.subscribe();

        assert!(
            !receiver
                .has_changed()
                .expect("publisher should keep subscriptions open"),
            "a new subscription must not replay a revision that predates it",
        );
    }

    fn assert_single_writer_type_is_send(_: &TrustedDesktopPublisher) {
        fn assert_send<T: Send>() {}
        assert_send::<TrustedDesktopPublisher>();
    }

    #[test]
    fn publisher_can_be_owned_by_one_watcher_thread() {
        let (publisher, _source) = trusted_desktop_cache(DesktopKind::Default);

        assert_single_writer_type_is_send(&publisher);
    }
}

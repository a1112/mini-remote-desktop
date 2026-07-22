//! Platform-neutral adapter for the native attended-consent surface.

use crate::consent::{
    ConsentAbortReason, ConsentBackend, ConsentBackendDecision, ConsentBackendFuture, ConsentPrompt,
};
use mrd_agent_ipc::PeerBinding;
use mrd_proto::SessionId;
use mrd_session::{PermissionScope, PermissionScopes};
use std::{
    num::NonZeroU64,
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
};
use tokio::sync::{oneshot, watch};

/// Maximum rendered session-identifier length, measured in UTF-16 code units.
pub(crate) const MAX_SESSION_ID_UTF16: usize = 96;
/// Maximum rendered device-identifier length, measured in UTF-16 code units.
pub(crate) const MAX_DEVICE_ID_UTF16: usize = 96;

/// A fixed, local label paired with a requested permission scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ConsentSurfaceScope {
    scope: PermissionScope,
    label: &'static str,
}

impl ConsentSurfaceScope {
    /// Permission represented by this checkbox.
    pub(crate) fn scope(self) -> PermissionScope {
        self.scope
    }

    /// Fixed local label rendered beside this checkbox.
    pub(crate) fn label(self) -> &'static str {
        self.label
    }
}

/// Sanitized, display-only data accepted by a native consent surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConsentSurfaceModel {
    session_id_utf16: Vec<u16>,
    device_id_utf16: Vec<u16>,
    peer_fingerprint: [u8; 32],
    scopes: Vec<ConsentSurfaceScope>,
}

impl ConsentSurfaceModel {
    fn from_display_parts(
        session_id: &SessionId,
        peer: &PeerBinding,
        requested_scopes: &PermissionScopes,
    ) -> Self {
        Self {
            session_id_utf16: sanitize_utf16(&session_id.0, MAX_SESSION_ID_UTF16),
            device_id_utf16: sanitize_utf16(&peer.device_id.0, MAX_DEVICE_ID_UTF16),
            peer_fingerprint: peer.key_id,
            scopes: requested_scopes
                .iter()
                .copied()
                .map(|scope| ConsentSurfaceScope {
                    scope,
                    label: scope_label(scope),
                })
                .collect(),
        }
    }

    /// Sanitized session identifier without a trailing NUL.
    pub(crate) fn session_id_utf16(&self) -> &[u16] {
        &self.session_id_utf16
    }

    /// Sanitized device identifier without a trailing NUL.
    pub(crate) fn device_id_utf16(&self) -> &[u16] {
        &self.device_id_utf16
    }

    /// Complete authenticated peer-key fingerprint.
    pub(crate) fn peer_fingerprint(&self) -> &[u8; 32] {
        &self.peer_fingerprint
    }

    /// Stable, fixed-label checkbox models for the requested scopes.
    pub(crate) fn scopes(&self) -> &[ConsentSurfaceScope] {
        &self.scopes
    }
}

/// A terminal user decision reported together with destruction of the surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ConsentSurfaceDecision {
    /// The user selected a non-empty subset and activated Allow.
    Approved(PermissionScopes),
    /// The user explicitly activated Deny.
    Denied,
    /// The surface closed or failed without a valid affirmative decision.
    Dismissed,
}

/// Coarse failure returned when a surface cannot be admitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConsentSurfaceError {
    /// The driver worker is not available.
    Unavailable,
    /// The driver's bounded command broker is full.
    Busy,
    /// The driver worker disconnected from its command broker.
    Disconnected,
}

/// Completion capability held by exactly one driver-owned surface request.
pub(crate) struct ConsentSurfaceCompletion {
    generation: NonZeroU64,
    slot: Arc<PromptSlot>,
    result: Option<oneshot::Sender<ConsentSurfaceDecision>>,
}

impl ConsentSurfaceCompletion {
    fn finish(mut self, decision: ConsentSurfaceDecision) {
        if self.slot.release(self.generation) {
            if let Some(result) = self.result.take() {
                let _ = result.send(decision);
            }
        }
    }
}

impl Drop for ConsentSurfaceCompletion {
    fn drop(&mut self) {
        self.slot.release(self.generation);
    }
}

/// Generation-bound request admitted to the native surface driver.
pub(crate) struct ConsentSurfaceRequest {
    generation: NonZeroU64,
    model: ConsentSurfaceModel,
    completion: ConsentSurfaceCompletion,
}

impl ConsentSurfaceRequest {
    /// Nonzero identity binding every native command to this request.
    pub(crate) fn generation(&self) -> u64 {
        self.generation.get()
    }

    /// Sanitized display-only model.
    pub(crate) fn model(&self) -> &ConsentSurfaceModel {
        &self.model
    }

    /// Report a terminal decision only after the surface is fully destroyed.
    pub(crate) fn finish_destroyed(self, decision: ConsentSurfaceDecision) {
        self.completion.finish(decision);
    }
}

/// Native surface command boundary implemented by the Windows UI worker.
pub(crate) trait ConsentSurfaceDriver: Send + Sync {
    /// Admit one generation-bound surface without blocking.
    fn try_show(&self, request: ConsentSurfaceRequest) -> Result<(), ConsentSurfaceError>;
    /// Request closure of the exact generation, if it is still active.
    fn request_close(&self, generation: u64);
    /// Stop and synchronously reclaim the driver worker.
    /// Implementations must be idempotent and must not panic.
    fn shutdown(&self);
}

struct PromptSlot {
    state: Mutex<PromptSlotState>,
}

struct PromptSlotState {
    active_generation: Option<NonZeroU64>,
    close_requested: bool,
}

impl PromptSlot {
    fn new() -> Self {
        Self {
            state: Mutex::new(PromptSlotState {
                active_generation: None,
                close_requested: false,
            }),
        }
    }

    fn acquire(&self, generation: NonZeroU64) -> bool {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if state.active_generation.is_some() {
            return false;
        }
        state.active_generation = Some(generation);
        state.close_requested = false;
        true
    }

    fn release(&self, generation: NonZeroU64) -> bool {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if state.active_generation != Some(generation) {
            return false;
        }
        state.active_generation = None;
        state.close_requested = false;
        true
    }

    fn active_generation(&self) -> Option<NonZeroU64> {
        self.state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .active_generation
    }

    fn mark_close_requested(&self, generation: NonZeroU64) -> bool {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if state.active_generation != Some(generation) || state.close_requested {
            return false;
        }
        state.close_requested = true;
        true
    }
}

struct NativeConsentCore {
    driver: Arc<dyn ConsentSurfaceDriver>,
    availability: Arc<AtomicBool>,
    next_generation: AtomicU64,
    slot: Arc<PromptSlot>,
    shutdown_complete: AtomicBool,
    shutdown_gate: Mutex<()>,
}

impl NativeConsentCore {
    fn allocate_generation(&self) -> Option<NonZeroU64> {
        if !self.availability.load(Ordering::Acquire) {
            return None;
        }
        match self
            .next_generation
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                current.checked_add(1).filter(|next| *next != 0)
            }) {
            Ok(current) => NonZeroU64::new(current),
            Err(_) => {
                self.availability.store(false, Ordering::Release);
                None
            }
        }
    }

    fn request_close_once(&self, generation: NonZeroU64) {
        let _gate = self
            .shutdown_gate
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if self.shutdown_complete.load(Ordering::Acquire)
            || !self.slot.mark_close_requested(generation)
        {
            return;
        }
        if catch_unwind(AssertUnwindSafe(|| {
            self.driver.request_close(generation.get());
        }))
        .is_err()
        {
            self.availability.store(false, Ordering::Release);
            self.shutdown_driver_locked();
        }
    }

    fn shutdown(&self) {
        let _gate = self
            .shutdown_gate
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if self.shutdown_complete.load(Ordering::Acquire) {
            return;
        }
        self.availability.store(false, Ordering::Release);
        if let Some(generation) = self.slot.active_generation() {
            if self.slot.mark_close_requested(generation) {
                let _ = catch_unwind(AssertUnwindSafe(|| {
                    self.driver.request_close(generation.get());
                }));
            }
        }
        self.shutdown_driver_locked();
    }

    fn shutdown_driver_locked(&self) {
        const MAX_SHUTDOWN_ATTEMPTS: usize = 2;
        for _ in 0..MAX_SHUTDOWN_ATTEMPTS {
            if catch_unwind(AssertUnwindSafe(|| self.driver.shutdown())).is_ok() {
                self.shutdown_complete.store(true, Ordering::Release);
                return;
            }
        }
        // Keep the state retryable. A later backend/core shutdown attempt may
        // still reclaim a driver that violated the no-panic contract.
        self.shutdown_complete.store(false, Ordering::Release);
    }
}

impl Drop for NativeConsentCore {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Bounded, cancellation-safe native implementation of [`ConsentBackend`].
pub(crate) struct NativeConsentBackend {
    core: Arc<NativeConsentCore>,
}

impl NativeConsentBackend {
    /// Construct an adapter over a native surface driver and its atomic health bit.
    pub(crate) fn new(
        driver: Arc<dyn ConsentSurfaceDriver>,
        availability: Arc<AtomicBool>,
    ) -> Self {
        Self {
            core: Arc::new(NativeConsentCore {
                driver,
                availability,
                next_generation: AtomicU64::new(1),
                slot: Arc::new(PromptSlot::new()),
                shutdown_complete: AtomicBool::new(false),
                shutdown_gate: Mutex::new(()),
            }),
        }
    }

    fn prompt_parts(
        &self,
        session_id: SessionId,
        peer: PeerBinding,
        requested_scopes: PermissionScopes,
        abort: watch::Receiver<Option<ConsentAbortReason>>,
    ) -> ConsentBackendFuture {
        let core = Arc::clone(&self.core);
        Box::pin(async move { run_prompt(core, session_id, peer, requested_scopes, abort).await })
    }

    #[cfg(test)]
    fn set_next_generation(&self, generation: u64) {
        self.core
            .next_generation
            .store(generation, Ordering::Release);
    }
}

impl Drop for NativeConsentBackend {
    fn drop(&mut self) {
        self.core.shutdown();
    }
}

impl ConsentBackend for NativeConsentBackend {
    fn is_available(&self) -> bool {
        self.core.availability.load(Ordering::Acquire)
    }

    fn prompt(
        &self,
        prompt: ConsentPrompt,
        abort: watch::Receiver<Option<ConsentAbortReason>>,
    ) -> ConsentBackendFuture {
        let (session_id, peer, requested_scopes) = prompt.into_display_parts();
        self.prompt_parts(session_id, peer, requested_scopes, abort)
    }
}

struct PromptCloseGuard {
    core: Arc<NativeConsentCore>,
    generation: NonZeroU64,
    close_requested: bool,
    armed: bool,
}

impl PromptCloseGuard {
    fn close_once(&mut self) {
        if self.armed && !self.close_requested {
            self.close_requested = true;
            self.core.request_close_once(self.generation);
        }
    }

    fn fail_stop(&mut self) {
        if self.armed {
            self.close_requested = true;
            self.core.shutdown();
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for PromptCloseGuard {
    fn drop(&mut self) {
        self.close_once();
    }
}

async fn run_prompt(
    core: Arc<NativeConsentCore>,
    session_id: SessionId,
    peer: PeerBinding,
    requested_scopes: PermissionScopes,
    mut abort: watch::Receiver<Option<ConsentAbortReason>>,
) -> ConsentBackendDecision {
    if abort.borrow().is_some() || abort.has_changed().is_err() {
        return ConsentBackendDecision::Cancelled;
    }
    let Some(generation) = core.allocate_generation() else {
        return ConsentBackendDecision::Dismissed;
    };
    if !core.slot.acquire(generation) {
        return ConsentBackendDecision::Dismissed;
    }

    let model = ConsentSurfaceModel::from_display_parts(&session_id, &peer, &requested_scopes);
    let (result, mut destroyed) = oneshot::channel();
    let request = ConsentSurfaceRequest {
        generation,
        model,
        completion: ConsentSurfaceCompletion {
            generation,
            slot: Arc::clone(&core.slot),
            result: Some(result),
        },
    };
    let mut guard = PromptCloseGuard {
        core: Arc::clone(&core),
        generation,
        close_requested: false,
        armed: true,
    };
    let shown = catch_unwind(AssertUnwindSafe(|| core.driver.try_show(request)));
    match shown {
        Ok(Ok(())) => {}
        Ok(Err(_)) => {
            guard.disarm();
            return ConsentBackendDecision::Dismissed;
        }
        Err(_) => {
            guard.fail_stop();
            let _ = destroyed.await;
            guard.disarm();
            return ConsentBackendDecision::Dismissed;
        }
    }
    let mut cancelled = false;
    loop {
        tokio::select! {
            biased;
            changed = abort.changed(), if !cancelled => {
                if changed.is_err() || abort.borrow_and_update().is_some() {
                    cancelled = true;
                    guard.close_once();
                }
            }
            outcome = &mut destroyed => {
                guard.disarm();
                if cancelled {
                    return ConsentBackendDecision::Cancelled;
                }
                return normalize_surface_decision(outcome, &requested_scopes);
            }
        }
    }
}

fn normalize_surface_decision(
    outcome: Result<ConsentSurfaceDecision, oneshot::error::RecvError>,
    requested_scopes: &PermissionScopes,
) -> ConsentBackendDecision {
    match outcome {
        Ok(ConsentSurfaceDecision::Approved(scopes))
            if !scopes.is_empty() && scopes.is_subset(requested_scopes) =>
        {
            ConsentBackendDecision::Approved(scopes)
        }
        Ok(ConsentSurfaceDecision::Denied) => ConsentBackendDecision::Denied,
        Ok(ConsentSurfaceDecision::Approved(_) | ConsentSurfaceDecision::Dismissed) | Err(_) => {
            ConsentBackendDecision::Dismissed
        }
    }
}

fn sanitize_utf16(value: &str, maximum_units: usize) -> Vec<u16> {
    if maximum_units == 0 {
        return Vec::new();
    }
    let mut rendered = Vec::with_capacity(maximum_units.min(value.len()));
    let mut boundaries = Vec::new();
    let mut truncated = false;
    for character in value.chars() {
        let character = if forbidden_display_character(character) {
            '\u{fffd}'
        } else {
            character
        };
        let mut units = [0_u16; 2];
        let encoded = character.encode_utf16(&mut units);
        if rendered.len() + encoded.len() > maximum_units {
            truncated = true;
            break;
        }
        boundaries.push(rendered.len());
        rendered.extend_from_slice(encoded);
    }
    if truncated {
        while rendered.len() == maximum_units {
            let Some(start) = boundaries.pop() else {
                rendered.clear();
                break;
            };
            rendered.truncate(start);
        }
        rendered.push('…' as u16);
    }
    rendered
}

fn forbidden_display_character(character: char) -> bool {
    character.is_control()
        || matches!(
            character,
            '\u{00ad}'
                | '\u{0600}'..='\u{0605}'
                | '\u{061c}'
                | '\u{06dd}'
                | '\u{070f}'
                | '\u{0890}'..='\u{0891}'
                | '\u{08e2}'
                | '\u{180e}'
                | '\u{200b}'..='\u{200f}'
                | '\u{202a}'..='\u{202e}'
                | '\u{2060}'..='\u{2064}'
                | '\u{2066}'..='\u{206f}'
                | '\u{feff}'
                | '\u{fff9}'..='\u{fffb}'
                | '\u{110bd}'
                | '\u{110cd}'
                | '\u{13430}'..='\u{1343f}'
                | '\u{1bca0}'..='\u{1bca3}'
                | '\u{1d173}'..='\u{1d17a}'
                | '\u{e0001}'
                | '\u{e0020}'..='\u{e007f}'
        )
}

fn scope_label(scope: PermissionScope) -> &'static str {
    match scope {
        PermissionScope::ScreenView => "View the screen",
        PermissionScope::InputPointer => "Control the pointer",
        PermissionScope::InputKeyboard => "Use the keyboard",
        PermissionScope::ClipboardRead => "Read the clipboard",
        PermissionScope::ClipboardWrite => "Write to the clipboard",
        PermissionScope::FileRead => "Read files",
        PermissionScope::FileWrite => "Write files",
        PermissionScope::AudioListen => "Listen to audio",
        PermissionScope::AudioTalk => "Use the microphone",
        PermissionScope::DisplaySwitch => "Switch displays",
        PermissionScope::DisplayMultiView => "View multiple displays",
        PermissionScope::PowerRestart => "Restart this device",
        PermissionScope::PowerShutdown => "Shut down this device",
        PermissionScope::TerminalOpen => "Open a terminal",
        PermissionScope::PrivacyBlockLocalInput => "Block local input",
        PermissionScope::PrivacyBlankScreen => "Blank the local screen",
        PermissionScope::SecureDesktopView => "View secure desktops",
        PermissionScope::SecureDesktopControl => "Control secure desktops",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mrd_proto::DeviceId;
    use std::{
        future::Future,
        pin::Pin,
        sync::{atomic::AtomicUsize, Mutex},
        task::{Context, Poll, Waker},
    };

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum DriverEvent {
        Show(u64),
        Close(u64),
        Shutdown,
    }

    #[derive(Default)]
    struct FakeDriver {
        shows: AtomicUsize,
        closes: Mutex<Vec<u64>>,
        requests: Mutex<Vec<ConsentSurfaceRequest>>,
        fail_next_show: AtomicBool,
        panic_next_show: AtomicBool,
        panic_next_close: AtomicBool,
        panic_shutdowns_remaining: AtomicUsize,
        shutdowns: AtomicUsize,
        events: Mutex<Vec<DriverEvent>>,
    }

    impl ConsentSurfaceDriver for FakeDriver {
        fn try_show(&self, request: ConsentSurfaceRequest) -> Result<(), ConsentSurfaceError> {
            if self.fail_next_show.swap(false, Ordering::SeqCst) {
                return Err(ConsentSurfaceError::Busy);
            }
            self.shows.fetch_add(1, Ordering::SeqCst);
            let generation = request.generation();
            self.requests.lock().unwrap().push(request);
            self.events
                .lock()
                .unwrap()
                .push(DriverEvent::Show(generation));
            if self.panic_next_show.swap(false, Ordering::SeqCst) {
                panic!("injected try_show panic after retaining the request");
            }
            Ok(())
        }

        fn request_close(&self, generation: u64) {
            self.closes.lock().unwrap().push(generation);
            self.events
                .lock()
                .unwrap()
                .push(DriverEvent::Close(generation));
            if self.panic_next_close.swap(false, Ordering::SeqCst) {
                panic!("injected request_close panic");
            }
        }

        fn shutdown(&self) {
            self.shutdowns.fetch_add(1, Ordering::SeqCst);
            self.events.lock().unwrap().push(DriverEvent::Shutdown);
            if self
                .panic_shutdowns_remaining
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                    remaining.checked_sub(1)
                })
                .is_ok()
            {
                panic!("injected shutdown panic before request reclamation");
            }
            self.requests.lock().unwrap().clear();
        }
    }

    impl FakeDriver {
        fn generation(&self) -> u64 {
            self.requests.lock().unwrap()[0].generation()
        }

        fn destroy(&self, generation: u64, decision: ConsentSurfaceDecision) -> bool {
            let mut requests = self.requests.lock().unwrap();
            let Some(index) = requests
                .iter()
                .position(|request| request.generation() == generation)
            else {
                return false;
            };
            let request = requests.remove(index);
            drop(requests);
            request.finish_destroyed(decision);
            true
        }

        fn worker_exit(&self, availability: &AtomicBool) {
            availability.store(false, Ordering::Release);
            self.requests.lock().unwrap().clear();
        }
    }

    fn fixture() -> (
        NativeConsentBackend,
        Arc<FakeDriver>,
        watch::Sender<Option<ConsentAbortReason>>,
    ) {
        let driver = Arc::new(FakeDriver::default());
        let available = Arc::new(AtomicBool::new(true));
        let backend = NativeConsentBackend::new(driver.clone(), available);
        let (abort, _) = watch::channel(None);
        (backend, driver, abort)
    }

    fn prompt(
        backend: &NativeConsentBackend,
        abort: watch::Receiver<Option<ConsentAbortReason>>,
    ) -> ConsentBackendFuture {
        backend.prompt_parts(
            SessionId("session".into()),
            PeerBinding {
                device_id: DeviceId("device".into()),
                key_id: [0xabu8; 32],
            },
            [PermissionScope::ScreenView].into_iter().collect(),
            abort,
        )
    }

    fn poll_once(future: &mut ConsentBackendFuture) -> Poll<ConsentBackendDecision> {
        let waker = Waker::noop();
        Future::poll(Pin::as_mut(future), &mut Context::from_waker(waker))
    }

    fn scopes(values: &[PermissionScope]) -> PermissionScopes {
        values.iter().copied().collect()
    }

    fn prompt_with_scopes(
        backend: &NativeConsentBackend,
        abort: watch::Receiver<Option<ConsentAbortReason>>,
        requested_scopes: PermissionScopes,
    ) -> ConsentBackendFuture {
        backend.prompt_parts(
            SessionId("session".into()),
            PeerBinding {
                device_id: DeviceId("device".into()),
                key_id: [0xabu8; 32],
            },
            requested_scopes,
            abort,
        )
    }

    #[test]
    fn prompt_has_no_generation_or_driver_side_effect_before_first_poll() {
        let (backend, driver, abort) = fixture();
        backend.set_next_generation(77);
        let future = prompt(&backend, abort.subscribe());
        assert_eq!(driver.shows.load(Ordering::SeqCst), 0);
        assert!(driver.requests.lock().unwrap().is_empty());
        drop(future);
        assert_eq!(driver.shows.load(Ordering::SeqCst), 0);
        assert!(driver.closes.lock().unwrap().is_empty());

        let mut next = prompt(&backend, abort.subscribe());
        assert_eq!(poll_once(&mut next), Poll::Pending);
        assert_eq!(driver.generation(), 77, "an unpolled prompt is fully lazy");
    }

    #[test]
    fn first_poll_admits_exactly_one_nonzero_generation() {
        let (backend, driver, abort) = fixture();
        let mut future = prompt(&backend, abort.subscribe());
        assert_eq!(poll_once(&mut future), Poll::Pending);
        assert_eq!(driver.shows.load(Ordering::SeqCst), 1);
        let requests = driver.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_ne!(requests[0].generation(), 0);
    }

    #[test]
    fn preexisting_abort_completes_without_showing_a_surface() {
        let (backend, driver, abort) = fixture();
        abort.send_replace(Some(ConsentAbortReason::DesktopChanged));
        let mut future = prompt(&backend, abort.subscribe());
        assert_eq!(
            poll_once(&mut future),
            Poll::Ready(ConsentBackendDecision::Cancelled)
        );
        assert_eq!(driver.shows.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn one_surface_remains_bounded_until_its_destroy_ack() {
        let (backend, driver, abort) = fixture();
        let mut first = prompt(&backend, abort.subscribe());
        assert_eq!(poll_once(&mut first), Poll::Pending);
        let first_generation = driver.generation();

        let mut second = prompt(&backend, abort.subscribe());
        assert_eq!(
            poll_once(&mut second),
            Poll::Ready(ConsentBackendDecision::Dismissed)
        );
        assert_eq!(driver.shows.load(Ordering::SeqCst), 1);
        assert_eq!(poll_once(&mut first), Poll::Pending);

        assert!(driver.destroy(
            first_generation,
            ConsentSurfaceDecision::Approved(scopes(&[PermissionScope::ScreenView]))
        ));
        assert_eq!(
            poll_once(&mut first),
            Poll::Ready(ConsentBackendDecision::Approved(scopes(&[
                PermissionScope::ScreenView,
            ])))
        );

        let mut third = prompt(&backend, abort.subscribe());
        assert_eq!(poll_once(&mut third), Poll::Pending);
        assert_ne!(driver.generation(), first_generation);
    }

    #[test]
    fn exact_and_subset_approvals_are_preserved_after_destruction() {
        let (backend, driver, abort) = fixture();
        let requested = scopes(&[
            PermissionScope::ScreenView,
            PermissionScope::InputPointer,
            PermissionScope::InputKeyboard,
        ]);
        for approved in [requested.clone(), scopes(&[PermissionScope::InputPointer])] {
            let mut future = prompt_with_scopes(&backend, abort.subscribe(), requested.clone());
            assert_eq!(poll_once(&mut future), Poll::Pending);
            let generation = driver.generation();
            assert!(driver.destroy(
                generation,
                ConsentSurfaceDecision::Approved(approved.clone())
            ));
            assert_eq!(
                poll_once(&mut future),
                Poll::Ready(ConsentBackendDecision::Approved(approved))
            );
        }
    }

    #[test]
    fn empty_and_escalated_approvals_fail_closed() {
        let (backend, driver, abort) = fixture();
        for invalid in [
            PermissionScopes::new(),
            scopes(&[PermissionScope::ScreenView, PermissionScope::PowerShutdown]),
        ] {
            let mut future = prompt(&backend, abort.subscribe());
            assert_eq!(poll_once(&mut future), Poll::Pending);
            let generation = driver.generation();
            assert!(driver.destroy(generation, ConsentSurfaceDecision::Approved(invalid)));
            assert_eq!(
                poll_once(&mut future),
                Poll::Ready(ConsentBackendDecision::Dismissed)
            );
        }
    }

    #[test]
    fn deny_and_dismiss_are_delivered_only_after_destruction() {
        let (backend, driver, abort) = fixture();
        for (surface, backend_decision) in [
            (
                ConsentSurfaceDecision::Denied,
                ConsentBackendDecision::Denied,
            ),
            (
                ConsentSurfaceDecision::Dismissed,
                ConsentBackendDecision::Dismissed,
            ),
        ] {
            let mut future = prompt(&backend, abort.subscribe());
            assert_eq!(poll_once(&mut future), Poll::Pending);
            assert_eq!(poll_once(&mut future), Poll::Pending);
            let generation = driver.generation();
            assert!(driver.destroy(generation, surface));
            assert_eq!(poll_once(&mut future), Poll::Ready(backend_decision));
        }
    }

    #[test]
    fn abort_requests_one_exact_close_and_waits_for_destruction() {
        let (backend, driver, abort) = fixture();
        let mut future = prompt(&backend, abort.subscribe());
        assert_eq!(poll_once(&mut future), Poll::Pending);
        let generation = driver.generation();
        abort.send_replace(Some(ConsentAbortReason::DesktopChanged));
        assert_eq!(poll_once(&mut future), Poll::Pending);
        assert_eq!(driver.closes.lock().unwrap().as_slice(), &[generation]);
        assert_eq!(poll_once(&mut future), Poll::Pending);
        assert!(driver.destroy(
            generation,
            ConsentSurfaceDecision::Approved(scopes(&[PermissionScope::ScreenView]))
        ));
        assert_eq!(
            poll_once(&mut future),
            Poll::Ready(ConsentBackendDecision::Cancelled)
        );
        assert_eq!(driver.closes.lock().unwrap().as_slice(), &[generation]);
    }

    #[test]
    fn closing_the_abort_watch_is_also_a_fail_closed_cancellation() {
        let (backend, driver, abort) = fixture();
        let mut future = prompt(&backend, abort.subscribe());
        assert_eq!(poll_once(&mut future), Poll::Pending);
        let generation = driver.generation();
        drop(abort);
        assert_eq!(poll_once(&mut future), Poll::Pending);
        assert_eq!(driver.closes.lock().unwrap().as_slice(), &[generation]);
        assert!(driver.destroy(generation, ConsentSurfaceDecision::Denied));
        assert_eq!(
            poll_once(&mut future),
            Poll::Ready(ConsentBackendDecision::Cancelled)
        );
    }

    #[test]
    fn dropping_a_polled_future_closes_once_and_keeps_the_slot_until_destroyed() {
        let (backend, driver, abort) = fixture();
        let mut first = prompt(&backend, abort.subscribe());
        assert_eq!(poll_once(&mut first), Poll::Pending);
        let generation = driver.generation();
        drop(first);
        assert_eq!(driver.closes.lock().unwrap().as_slice(), &[generation]);

        let mut blocked = prompt(&backend, abort.subscribe());
        assert_eq!(
            poll_once(&mut blocked),
            Poll::Ready(ConsentBackendDecision::Dismissed)
        );
        assert!(driver.destroy(generation, ConsentSurfaceDecision::Dismissed));

        let mut next = prompt(&backend, abort.subscribe());
        assert_eq!(poll_once(&mut next), Poll::Pending);
        assert_ne!(driver.generation(), generation);
    }

    #[test]
    fn driver_rejection_releases_the_slot_and_fails_closed() {
        let (backend, driver, abort) = fixture();
        driver.fail_next_show.store(true, Ordering::SeqCst);
        let mut rejected = prompt(&backend, abort.subscribe());
        assert_eq!(
            poll_once(&mut rejected),
            Poll::Ready(ConsentBackendDecision::Dismissed)
        );

        let mut next = prompt(&backend, abort.subscribe());
        assert_eq!(poll_once(&mut next), Poll::Pending);
    }

    #[test]
    fn worker_exit_reclaims_the_request_and_marks_the_backend_unavailable() {
        let (backend, driver, abort) = fixture();
        let mut active = prompt(&backend, abort.subscribe());
        assert_eq!(poll_once(&mut active), Poll::Pending);
        driver.worker_exit(&backend.core.availability);
        assert_eq!(
            poll_once(&mut active),
            Poll::Ready(ConsentBackendDecision::Dismissed)
        );
        assert!(!backend.is_available());

        let mut next = prompt(&backend, abort.subscribe());
        assert_eq!(
            poll_once(&mut next),
            Poll::Ready(ConsentBackendDecision::Dismissed)
        );
        assert_eq!(driver.shows.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn try_show_panic_marks_unavailable_and_shutdown_reclaims_the_request() {
        let (backend, driver, abort) = fixture();
        driver.panic_next_show.store(true, Ordering::SeqCst);

        let mut future = prompt(&backend, abort.subscribe());
        assert_eq!(
            poll_once(&mut future),
            Poll::Ready(ConsentBackendDecision::Dismissed)
        );
        assert!(!backend.is_available());
        assert!(driver.requests.lock().unwrap().is_empty());
        assert_eq!(driver.shutdowns.load(Ordering::SeqCst), 1);

        let mut next = prompt(&backend, abort.subscribe());
        assert_eq!(
            poll_once(&mut next),
            Poll::Ready(ConsentBackendDecision::Dismissed)
        );
        assert_eq!(driver.shows.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn request_close_panic_marks_unavailable_and_shutdown_reclaims_the_request() {
        let (backend, driver, abort) = fixture();
        let mut future = prompt(&backend, abort.subscribe());
        assert_eq!(poll_once(&mut future), Poll::Pending);
        let generation = driver.generation();
        driver.panic_next_close.store(true, Ordering::SeqCst);

        abort.send_replace(Some(ConsentAbortReason::DesktopChanged));
        assert_eq!(
            poll_once(&mut future),
            Poll::Ready(ConsentBackendDecision::Cancelled)
        );
        assert!(!backend.is_available());
        assert!(driver.requests.lock().unwrap().is_empty());
        assert_eq!(driver.shutdowns.load(Ordering::SeqCst), 1);
        assert_eq!(driver.closes.lock().unwrap().as_slice(), &[generation]);
    }

    #[test]
    fn unavailable_and_exhausted_generation_fail_without_reuse() {
        let driver = Arc::new(FakeDriver::default());
        let available = Arc::new(AtomicBool::new(false));
        let backend = NativeConsentBackend::new(driver.clone(), available.clone());
        let (_abort, receiver) = watch::channel(None);
        let mut unavailable = prompt(&backend, receiver);
        assert_eq!(
            poll_once(&mut unavailable),
            Poll::Ready(ConsentBackendDecision::Dismissed)
        );
        assert_eq!(driver.shows.load(Ordering::SeqCst), 0);

        available.store(true, Ordering::SeqCst);
        backend.set_next_generation(u64::MAX);
        let (_abort, receiver) = watch::channel(None);
        let mut exhausted = prompt(&backend, receiver);
        assert_eq!(
            poll_once(&mut exhausted),
            Poll::Ready(ConsentBackendDecision::Dismissed)
        );
        assert!(!backend.is_available());
        assert_eq!(driver.shows.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn display_model_sanitizes_utf16_and_exposes_only_fixed_scope_metadata() {
        let all_scopes = scopes(&[
            PermissionScope::ScreenView,
            PermissionScope::InputPointer,
            PermissionScope::InputKeyboard,
            PermissionScope::ClipboardRead,
            PermissionScope::ClipboardWrite,
            PermissionScope::FileRead,
            PermissionScope::FileWrite,
            PermissionScope::AudioListen,
            PermissionScope::AudioTalk,
            PermissionScope::DisplaySwitch,
            PermissionScope::DisplayMultiView,
            PermissionScope::PowerRestart,
            PermissionScope::PowerShutdown,
            PermissionScope::TerminalOpen,
            PermissionScope::PrivacyBlockLocalInput,
            PermissionScope::PrivacyBlankScreen,
            PermissionScope::SecureDesktopView,
            PermissionScope::SecureDesktopControl,
        ]);
        let key_id = std::array::from_fn(|index| index as u8);
        let model = ConsentSurfaceModel::from_display_parts(
            &SessionId(format!("{}😀", "s".repeat(MAX_SESSION_ID_UTF16 - 1))),
            &PeerBinding {
                device_id: DeviceId("dev\0\n\u{061c}\u{202e}\u{2069}ice".into()),
                key_id,
            },
            &all_scopes,
        );
        assert_eq!(model.session_id_utf16().len(), MAX_SESSION_ID_UTF16);
        assert_eq!(
            *model.session_id_utf16().last().unwrap(),
            '…' as u16,
            "truncation must reserve a complete visible ellipsis"
        );
        let device = String::from_utf16(model.device_id_utf16()).unwrap();
        assert_eq!(device, "dev�����ice");
        assert!(device
            .chars()
            .all(|character| !forbidden_display_character(character)));
        assert_eq!(model.peer_fingerprint(), &key_id);
        let expected_scope_rows = [
            (PermissionScope::ScreenView, "View the screen"),
            (PermissionScope::InputPointer, "Control the pointer"),
            (PermissionScope::InputKeyboard, "Use the keyboard"),
            (PermissionScope::ClipboardRead, "Read the clipboard"),
            (PermissionScope::ClipboardWrite, "Write to the clipboard"),
            (PermissionScope::FileRead, "Read files"),
            (PermissionScope::FileWrite, "Write files"),
            (PermissionScope::AudioListen, "Listen to audio"),
            (PermissionScope::AudioTalk, "Use the microphone"),
            (PermissionScope::DisplaySwitch, "Switch displays"),
            (PermissionScope::DisplayMultiView, "View multiple displays"),
            (PermissionScope::PowerRestart, "Restart this device"),
            (PermissionScope::PowerShutdown, "Shut down this device"),
            (PermissionScope::TerminalOpen, "Open a terminal"),
            (PermissionScope::PrivacyBlockLocalInput, "Block local input"),
            (
                PermissionScope::PrivacyBlankScreen,
                "Blank the local screen",
            ),
            (PermissionScope::SecureDesktopView, "View secure desktops"),
            (
                PermissionScope::SecureDesktopControl,
                "Control secure desktops",
            ),
        ];
        assert_eq!(
            model
                .scopes()
                .iter()
                .map(|row| (row.scope(), row.label()))
                .collect::<Vec<_>>(),
            expected_scope_rows
        );
    }

    #[test]
    fn display_sanitizer_replaces_every_unicode_format_character() {
        let format_ranges = [
            (0x00ad, 0x00ad),
            (0x0600, 0x0605),
            (0x061c, 0x061c),
            (0x06dd, 0x06dd),
            (0x070f, 0x070f),
            (0x0890, 0x0891),
            (0x08e2, 0x08e2),
            (0x180e, 0x180e),
            (0x200b, 0x200f),
            (0x202a, 0x202e),
            (0x2060, 0x2064),
            (0x2066, 0x206f),
            (0xfeff, 0xfeff),
            (0xfff9, 0xfffb),
            (0x110bd, 0x110bd),
            (0x110cd, 0x110cd),
            (0x13430, 0x1343f),
            (0x1bca0, 0x1bca3),
            (0x1d173, 0x1d17a),
            (0xe0001, 0xe0001),
            (0xe0020, 0xe007f),
        ];
        let format_characters = format_ranges
            .into_iter()
            .flat_map(|(start, end)| start..=end)
            .map(|codepoint| char::from_u32(codepoint).unwrap())
            .collect::<String>();
        let sanitized = sanitize_utf16(&format_characters, usize::MAX);

        assert_eq!(sanitized.len(), format_characters.chars().count());
        assert!(sanitized.iter().all(|unit| *unit == '\u{fffd}' as u16));
    }

    #[test]
    fn dropping_an_idle_backend_synchronously_shuts_down_once() {
        let (backend, driver, abort) = fixture();
        let availability = Arc::clone(&backend.core.availability);
        let future = prompt(&backend, abort.subscribe());

        drop(backend);
        assert!(!availability.load(Ordering::Acquire));
        assert_eq!(driver.shutdowns.load(Ordering::SeqCst), 1);
        assert_eq!(
            driver.events.lock().unwrap().as_slice(),
            &[DriverEvent::Shutdown]
        );

        drop(future);
        assert_eq!(driver.shutdowns.load(Ordering::SeqCst), 1);
        assert_eq!(
            driver.events.lock().unwrap().as_slice(),
            &[DriverEvent::Shutdown]
        );
    }

    #[test]
    fn dropping_a_backend_with_an_active_surface_closes_then_shuts_down_immediately() {
        let (backend, driver, abort) = fixture();
        let availability = Arc::clone(&backend.core.availability);
        let mut future = prompt(&backend, abort.subscribe());
        assert_eq!(poll_once(&mut future), Poll::Pending);
        let generation = driver.generation();

        drop(backend);
        assert!(!availability.load(Ordering::Acquire));
        assert_eq!(
            driver.events.lock().unwrap().as_slice(),
            &[
                DriverEvent::Show(generation),
                DriverEvent::Close(generation),
                DriverEvent::Shutdown,
            ]
        );
        assert!(driver.requests.lock().unwrap().is_empty());
        assert_eq!(
            poll_once(&mut future),
            Poll::Ready(ConsentBackendDecision::Dismissed),
            "driver shutdown must reclaim the completion instead of stranding the future"
        );
        drop(future);
        assert_eq!(driver.shutdowns.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn shutdown_panic_before_reclamation_is_retried_without_stranding_the_prompt() {
        let (backend, driver, abort) = fixture();
        let mut future = prompt(&backend, abort.subscribe());
        assert_eq!(poll_once(&mut future), Poll::Pending);
        driver.panic_shutdowns_remaining.store(1, Ordering::SeqCst);

        drop(backend);

        assert_eq!(driver.shutdowns.load(Ordering::SeqCst), 2);
        assert!(driver.requests.lock().unwrap().is_empty());
        assert_eq!(
            poll_once(&mut future),
            Poll::Ready(ConsentBackendDecision::Dismissed)
        );
        drop(future);
        assert_eq!(driver.shutdowns.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn dropping_a_backend_with_a_closing_surface_does_not_duplicate_close() {
        let (backend, driver, abort) = fixture();
        let mut future = prompt(&backend, abort.subscribe());
        assert_eq!(poll_once(&mut future), Poll::Pending);
        let generation = driver.generation();
        abort.send_replace(Some(ConsentAbortReason::DesktopChanged));
        assert_eq!(poll_once(&mut future), Poll::Pending);

        drop(backend);
        assert_eq!(
            driver.events.lock().unwrap().as_slice(),
            &[
                DriverEvent::Show(generation),
                DriverEvent::Close(generation),
                DriverEvent::Shutdown,
            ]
        );
        assert_eq!(
            poll_once(&mut future),
            Poll::Ready(ConsentBackendDecision::Cancelled)
        );
        drop(future);
        assert_eq!(driver.shutdowns.load(Ordering::SeqCst), 1);
    }
}

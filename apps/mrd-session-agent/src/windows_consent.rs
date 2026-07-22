//! Native Windows attended-consent surface driver.

use crate::native_consent::{
    ConsentSurfaceDecision, ConsentSurfaceDriver, ConsentSurfaceError, ConsentSurfaceRequest,
};
use mrd_session::{PermissionScope, PermissionScopes};
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    ffi::c_void,
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering},
        mpsc::{self, SyncSender},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
};
use thiserror::Error;
use windows::core::{w, PCWSTR};
use windows::Win32::{
    Foundation::{
        CloseHandle, GetLastError, SetLastError, ERROR_BUSY, HANDLE, HINSTANCE, HWND, LPARAM,
        LRESULT, WAIT_FAILED, WAIT_OBJECT_0, WIN32_ERROR, WPARAM,
    },
    Graphics::Gdi::{GetStockObject, UpdateWindow, COLOR_WINDOW, DEFAULT_GUI_FONT, HBRUSH},
    System::{
        LibraryLoader::GetModuleHandleW,
        Threading::{CreateEventW, GetCurrentThreadId, SetEvent, INFINITE},
    },
    UI::{
        Input::{
            GetCurrentInputMessageSource,
            KeyboardAndMouse::{GetFocus, SetActiveWindow, SetFocus},
            IMDT_KEYBOARD, IMDT_MOUSE, IMDT_PEN, IMDT_TOUCH, IMDT_TOUCHPAD, IMO_HARDWARE,
            INPUT_MESSAGE_SOURCE,
        },
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetParent,
            GetWindowLongPtrW, IsDialogMessageW, IsWindow, LoadCursorW,
            MsgWaitForMultipleObjectsEx, PeekMessageW, PostQuitMessage, PostThreadMessageW,
            RegisterClassW, SendMessageW, SetForegroundWindow, SetWindowLongPtrW, ShowWindow,
            TranslateMessage, UnregisterClassW, BM_CLICK, BM_GETCHECK, BM_SETCHECK, BN_CLICKED,
            BS_AUTOCHECKBOX, BS_DEFPUSHBUTTON, BS_PUSHBUTTON, CS_HREDRAW, CS_VREDRAW,
            CW_USEDEFAULT, GWLP_USERDATA, HMENU, IDC_ARROW, MSG, MWMO_INPUTAVAILABLE, PM_NOREMOVE,
            PM_REMOVE, QS_ALLINPUT, SC_CLOSE, SW_SHOW, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP,
            WM_CLOSE, WM_COMMAND, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_NCCREATE,
            WM_NCDESTROY, WM_QUIT, WM_SETFONT, WM_SYSCOMMAND, WM_USER, WNDCLASSW, WS_CAPTION,
            WS_CHILD, WS_CLIPCHILDREN, WS_OVERLAPPED, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE,
        },
    },
};

const WM_MRD_WAKE_CONSENT: u32 = WM_APP + 0x351;
const WM_MRD_KEYBOARD_DECISION: u32 = WM_APP + 0x352;
#[cfg(test)]
const WM_MRD_QUERY_INITIAL_FOCUS: u32 = WM_APP + 0x353;
#[cfg(test)]
const WM_MRD_QUERY_CONTROL_AUTHORITY: u32 = WM_APP + 0x354;
#[cfg(test)]
const WM_MRD_QUERY_SURFACE_AUTHORITY: u32 = WM_APP + 0x355;
#[cfg(test)]
const WM_MRD_TEST_CONTROL_ACTION: u32 = WM_APP + 0x356;
#[cfg(test)]
const WM_MRD_TEST_SURFACE_DISMISS: u32 = WM_APP + 0x357;
const SHOW_BROKER_CAPACITY: usize = 1;
const DENY_CONTROL_ID: u16 = 0x5d01;
const ALLOW_CONTROL_ID: u16 = 0x5d02;
const FIRST_SCOPE_CONTROL_ID: u16 = 0x5d10;
const SURFACE_WIDTH: i32 = 780;
const CONTENT_WIDTH: i32 = 720;
const SCOPE_COLUMNS: usize = 2;
const SCOPE_COLUMN_WIDTH: i32 = 350;
const SCOPE_BASE_Y: i32 = 168;
const SCOPE_ROW_HEIGHT: i32 = 28;
const VK_ESCAPE_CODE: usize = 0x1b;
const VK_RETURN_CODE: usize = 0x0d;
const VK_SPACE_CODE: usize = 0x20;
static NEXT_WORKER_GENERATION: AtomicUsize = AtomicUsize::new(1);
static NEXT_SURFACE_AUTHORITY: AtomicUsize = AtomicUsize::new(1);

thread_local! {
    static CALLBACK_FAILED: Cell<bool> = const { Cell::new(false) };
    static RETIRED_SURFACES: RefCell<Vec<*mut SurfaceState>> = const { RefCell::new(Vec::new()) };
    static TRUSTED_CONTROL: Cell<Option<PromptAuthority>> = const { Cell::new(None) };
    static TRUSTED_DISPATCH: Cell<Option<PromptAuthority>> = const { Cell::new(None) };
    static TRUSTED_KEYBOARD: Cell<Option<PromptAuthority>> = const { Cell::new(None) };
    static TRUSTED_CLOSE: Cell<Option<PromptAuthority>> = const { Cell::new(None) };
    static TRUSTED_DESTROY: Cell<Option<PromptAuthority>> = const { Cell::new(None) };
    static TRUSTED_CREATE_STATE: Cell<usize> = const { Cell::new(0) };
}

/// Failure while establishing the native consent UI worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub(crate) enum WindowsConsentError {
    /// A worker thread or its message queue was unavailable.
    #[error("native consent worker is unavailable")]
    WorkerUnavailable,
    /// A required Win32 operation failed.
    #[error("native consent operation {operation} failed with status {status}")]
    Native {
        /// Stable operation name.
        operation: &'static str,
        /// Native status code.
        status: i32,
    },
}

#[derive(Debug, Clone, Copy)]
struct WorkerToken {
    thread_id: u32,
    generation: usize,
}

struct WakeEvent(HANDLE);

// Kernel event handles are process-wide synchronization objects. Every access
// is an atomic SetEvent/wait operation, and Arc keeps the handle open until the
// worker and all producers have released it.
unsafe impl Send for WakeEvent {}
unsafe impl Sync for WakeEvent {}

impl WakeEvent {
    fn new() -> Result<Self, WindowsConsentError> {
        unsafe { CreateEventW(None, false, false, None) }
            .map(Self)
            .map_err(|error| native_error("create consent wake event", error))
    }

    fn signal(&self) -> bool {
        unsafe { SetEvent(self.0) }.is_ok()
    }
}

impl Drop for WakeEvent {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}

struct DriverShared {
    availability: Arc<AtomicBool>,
    show: Arc<Mutex<VecDeque<ConsentSurfaceRequest>>>,
    close: Arc<CloseLatch>,
    shutdown: Arc<AtomicBool>,
    reclaim: Arc<AtomicUsize>,
    wake: Arc<WakeEvent>,
    failure: Arc<Mutex<Option<WindowsConsentError>>>,
    supervisor_thread_id: Arc<AtomicU32>,
    worker: WorkerToken,
}

struct WorkerContext {
    availability: Arc<AtomicBool>,
    show: Arc<Mutex<VecDeque<ConsentSurfaceRequest>>>,
    close: Arc<CloseLatch>,
    shutdown: Arc<AtomicBool>,
    reclaim: Arc<AtomicUsize>,
    wake: Arc<WakeEvent>,
}

/// One-worker Windows implementation of the native consent surface boundary.
pub(crate) struct WindowsConsentSurfaceDriver {
    shared: Arc<DriverShared>,
    join: Mutex<Option<JoinHandle<()>>>,
}

impl WindowsConsentSurfaceDriver {
    /// Start a dedicated UI worker and wait until its message queue is ready.
    pub(crate) fn start() -> Result<(Arc<Self>, Arc<AtomicBool>), WindowsConsentError> {
        let generation = next_worker_generation()?;
        let availability = Arc::new(AtomicBool::new(false));
        let close = Arc::new(CloseLatch::default());
        let shutdown = Arc::new(AtomicBool::new(false));
        let reclaim = Arc::new(AtomicUsize::new(0));
        let wake = Arc::new(WakeEvent::new()?);
        let failure = Arc::new(Mutex::new(None));
        let supervisor_thread_id = Arc::new(AtomicU32::new(0));
        let show = Arc::new(Mutex::new(VecDeque::with_capacity(SHOW_BROKER_CAPACITY)));
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let worker_context = Arc::new(WorkerContext {
            availability: Arc::clone(&availability),
            show: Arc::clone(&show),
            close: Arc::clone(&close),
            shutdown: Arc::clone(&shutdown),
            reclaim: Arc::clone(&reclaim),
            wake: Arc::clone(&wake),
        });
        let supervisor_context = Arc::clone(&worker_context);
        let ui_context = Arc::clone(&worker_context);
        let worker_availability = Arc::clone(&availability);
        let worker_failure = Arc::clone(&failure);
        let fallback = ready_tx.clone();
        let (worker_join_tx, worker_join_rx) = mpsc::sync_channel::<JoinHandle<()>>(1);
        let supervisor_availability = Arc::clone(&availability);
        let supervisor_reclaim = Arc::clone(&reclaim);
        let running_supervisor_thread_id = Arc::clone(&supervisor_thread_id);
        let supervisor = thread::Builder::new()
            .name("mrd-consent-supervisor".to_owned())
            .spawn(move || {
                running_supervisor_thread_id
                    .store(unsafe { GetCurrentThreadId() }, Ordering::Release);
                if let Ok(worker) = worker_join_rx.recv() {
                    let _ = worker.join();
                }
                supervisor_availability.store(false, Ordering::Release);
                dismiss_pending_requests(&supervisor_context);
                reclaim_surface_after_worker_exit(&supervisor_reclaim);
            })
            .map_err(|_| WindowsConsentError::WorkerUnavailable)?;
        let worker = match thread::Builder::new()
            .name("mrd-consent-ui".to_owned())
            .spawn(move || {
                let result = catch_unwind(AssertUnwindSafe(|| {
                    native_worker_main(generation, &ui_context, ready_tx)
                }));
                worker_availability.store(false, Ordering::Release);
                dismiss_pending_requests(&ui_context);
                let error = match result {
                    Ok(Ok(())) => return,
                    Ok(Err(error)) => error,
                    Err(_) => WindowsConsentError::WorkerUnavailable,
                };
                *worker_failure
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(error);
                let _ = fallback.try_send(Err(error));
            }) {
            Ok(worker) => worker,
            Err(_) => {
                drop(worker_join_tx);
                let _ = supervisor.join();
                return Err(WindowsConsentError::WorkerUnavailable);
            }
        };

        let token = match ready_rx.recv() {
            Ok(Ok(token)) => token,
            Ok(Err(error)) => {
                let _ = worker_join_tx.send(worker);
                drop(worker_join_tx);
                let _ = supervisor.join();
                return Err(error);
            }
            Err(_) => {
                let _ = worker_join_tx.send(worker);
                drop(worker_join_tx);
                let _ = supervisor.join();
                return Err(WindowsConsentError::WorkerUnavailable);
            }
        };
        if let Err(error) = worker_join_tx.send(worker) {
            let _ = error.0.join();
            drop(worker_join_tx);
            let _ = supervisor.join();
            dismiss_pending_requests(&worker_context);
            reclaim_surface_after_worker_exit(&reclaim);
            return Err(WindowsConsentError::WorkerUnavailable);
        }
        drop(worker_join_tx);
        let shared = Arc::new(DriverShared {
            availability: Arc::clone(&availability),
            show,
            close,
            shutdown,
            reclaim,
            wake,
            failure,
            supervisor_thread_id,
            worker: token,
        });
        Ok((
            Arc::new(Self {
                shared,
                join: Mutex::new(Some(supervisor)),
            }),
            availability,
        ))
    }

    fn stop_and_join(&self) {
        self.shared.shutdown.store(true, Ordering::Release);
        self.shared.availability.store(false, Ordering::Release);
        let _ = wake_worker(&self.shared);
        let current_thread = unsafe { GetCurrentThreadId() };
        if current_thread == self.shared.worker.thread_id
            || current_thread == self.shared.supervisor_thread_id.load(Ordering::Acquire)
        {
            // A completion waker may synchronously drop the backend. The
            // current owned thread cannot join itself; its peer supervisor
            // still observes shutdown and performs the eventual join/reclaim.
            return;
        }
        let mut join = self
            .join
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(join) = join.take() {
            let _ = join.join();
            reclaim_surface_after_worker_exit(&self.shared.reclaim);
        }
    }
}

impl ConsentSurfaceDriver for WindowsConsentSurfaceDriver {
    fn try_show(&self, request: ConsentSurfaceRequest) -> Result<(), ConsentSurfaceError> {
        if !self.shared.availability.load(Ordering::Acquire)
            || self.shared.shutdown.load(Ordering::Acquire)
        {
            return Err(ConsentSurfaceError::Unavailable);
        }
        let generation = request.generation();
        if !self.shared.close.reserve(generation) {
            return Err(ConsentSurfaceError::Busy);
        }
        let result = admit_show(
            &self.shared.show,
            request,
            || {
                self.shared.availability.load(Ordering::Acquire)
                    && !self.shared.shutdown.load(Ordering::Acquire)
            },
            || wake_worker(&self.shared),
        );
        if result.is_err() {
            self.shared.close.release(generation);
        }
        if result == Err(ConsentSurfaceError::Disconnected) {
            self.shared.availability.store(false, Ordering::Release);
        }
        result
    }

    fn request_close(&self, generation: u64) {
        self.shared.close.request(generation);
        if !wake_worker(&self.shared) {
            self.shared.availability.store(false, Ordering::Release);
            self.shared.shutdown.store(true, Ordering::Release);
        }
    }

    fn shutdown(&self) {
        self.stop_and_join();
    }
}

fn wake_worker(shared: &DriverShared) -> bool {
    shared.wake.signal()
        || unsafe {
            PostThreadMessageW(
                shared.worker.thread_id,
                WM_MRD_WAKE_CONSENT,
                WPARAM(shared.worker.generation),
                LPARAM(0),
            )
        }
        .is_ok()
}

/// Admit and wake as one ownership transaction. The queue lock prevents the
/// worker from taking the request until a failed wakeup has retracted it.
fn admit_show<T>(
    show: &Mutex<VecDeque<T>>,
    request: T,
    worker_accepting: impl FnOnce() -> bool,
    wake_worker: impl FnOnce() -> bool,
) -> Result<(), ConsentSurfaceError> {
    let mut show = match show.try_lock() {
        Ok(show) => show,
        Err(std::sync::TryLockError::WouldBlock) => return Err(ConsentSurfaceError::Busy),
        Err(std::sync::TryLockError::Poisoned(_)) => return Err(ConsentSurfaceError::Disconnected),
    };
    if !worker_accepting() {
        return Err(ConsentSurfaceError::Unavailable);
    }
    if show.len() >= SHOW_BROKER_CAPACITY {
        return Err(ConsentSurfaceError::Busy);
    }
    show.push_back(request);
    if wake_worker() {
        return Ok(());
    }
    drop(show.pop_back());
    Err(ConsentSurfaceError::Disconnected)
}

impl Drop for WindowsConsentSurfaceDriver {
    fn drop(&mut self) {
        self.stop_and_join();
    }
}

#[cfg(test)]
mod show_broker_tests {
    use super::admit_show;
    use crate::native_consent::ConsentSurfaceError;
    use std::cell::Cell;
    use std::collections::VecDeque;
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    };

    struct DropCounter(Arc<AtomicUsize>);

    impl Drop for DropCounter {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::AcqRel);
        }
    }

    #[test]
    fn failed_wakeup_retracts_and_drops_the_admitted_request() {
        let queue = Mutex::new(VecDeque::new());
        let drops = Arc::new(AtomicUsize::new(0));

        let result = admit_show(
            &queue,
            DropCounter(Arc::clone(&drops)),
            || true,
            || {
                assert!(
                    queue.try_lock().is_err(),
                    "wakeup must be inside the admission lock"
                );
                false
            },
        );

        assert_eq!(result, Err(ConsentSurfaceError::Disconnected));
        assert!(queue.lock().unwrap().is_empty());
        assert_eq!(drops.load(Ordering::Acquire), 1);
    }

    #[test]
    fn capacity_one_rejects_without_waking_or_replacing_the_admitted_request() {
        let queue = Mutex::new(VecDeque::new());
        assert_eq!(admit_show(&queue, 41_u64, || true, || true), Ok(()));
        let second_wakeup_called = Cell::new(false);

        let result = admit_show(
            &queue,
            42_u64,
            || true,
            || {
                second_wakeup_called.set(true);
                true
            },
        );

        assert_eq!(result, Err(ConsentSurfaceError::Busy));
        assert!(!second_wakeup_called.get());
        assert_eq!(
            queue.lock().unwrap().iter().copied().collect::<Vec<_>>(),
            [41]
        );
    }

    #[test]
    fn poisoned_broker_disconnects_without_waking() {
        let queue = Mutex::new(VecDeque::new());
        let _ = catch_unwind(AssertUnwindSafe(|| {
            let _guard = queue.lock().unwrap();
            panic!("inject queue poison");
        }));
        let wake_called = Cell::new(false);

        assert_eq!(
            admit_show(
                &queue,
                41_u64,
                || true,
                || {
                    wake_called.set(true);
                    true
                },
            ),
            Err(ConsentSurfaceError::Disconnected)
        );
        assert!(!wake_called.get());
    }

    #[test]
    fn worker_exit_observed_inside_admission_lock_rejects_without_waking() {
        let queue = Mutex::new(VecDeque::new());
        let drops = Arc::new(AtomicUsize::new(0));
        let wake_called = Cell::new(false);

        assert_eq!(
            admit_show(
                &queue,
                DropCounter(Arc::clone(&drops)),
                || false,
                || {
                    wake_called.set(true);
                    true
                },
            ),
            Err(ConsentSurfaceError::Unavailable)
        );
        assert!(!wake_called.get());
        assert!(queue.lock().unwrap().is_empty());
        assert_eq!(drops.load(Ordering::Acquire), 1);
    }
}

fn next_worker_generation() -> Result<usize, WindowsConsentError> {
    NEXT_WORKER_GENERATION
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            current.checked_add(1).filter(|next| *next != 0)
        })
        .map_err(|_| WindowsConsentError::WorkerUnavailable)
}

fn next_surface_authority() -> Result<usize, WindowsConsentError> {
    NEXT_SURFACE_AUTHORITY
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            current.checked_add(1).filter(|next| *next != 0)
        })
        .map_err(|_| WindowsConsentError::WorkerUnavailable)
}

struct WorkerResources {
    class_name: Vec<u16>,
    instance: HINSTANCE,
}

impl WorkerResources {
    fn register(generation: usize) -> Result<Self, WindowsConsentError> {
        let module = unsafe { GetModuleHandleW(None) }
            .map_err(|error| native_error("load consent module", error))?;
        let instance = HINSTANCE(module.0);
        let class_name = format!("MrdConsentSurface-{generation}\0")
            .encode_utf16()
            .collect::<Vec<_>>();
        let cursor = unsafe { LoadCursorW(None, IDC_ARROW) }
            .map_err(|error| native_error("load consent cursor", error))?;
        let class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(consent_window_proc),
            hInstance: instance,
            hCursor: cursor,
            hbrBackground: HBRUSH((COLOR_WINDOW.0 as usize + 1) as *mut c_void),
            lpszClassName: PCWSTR(class_name.as_ptr()),
            ..WNDCLASSW::default()
        };
        if unsafe { RegisterClassW(&class) } == 0 {
            return Err(last_native_error("register consent class"));
        }
        Ok(Self {
            class_name,
            instance,
        })
    }
}

impl Drop for WorkerResources {
    fn drop(&mut self) {
        drain_retired_surfaces();
        let _ = unsafe { UnregisterClassW(PCWSTR(self.class_name.as_ptr()), Some(self.instance)) };
    }
}

#[derive(Default)]
struct ActiveSurfaceGuard {
    active: Option<(HWND, u64)>,
}

impl ActiveSurfaceGuard {
    fn slot(&mut self) -> &mut Option<(HWND, u64)> {
        &mut self.active
    }
}

impl Drop for ActiveSurfaceGuard {
    fn drop(&mut self) {
        // This is the last worker-thread owner of the HWND. It also runs if
        // message dispatch or later teardown unwinds unexpectedly.
        close_active_surface(&mut self.active, ConsentSurfaceDecision::Dismissed);
    }
}

fn native_worker_main(
    generation: usize,
    context: &WorkerContext,
    ready: SyncSender<Result<WorkerToken, WindowsConsentError>>,
) -> Result<(), WindowsConsentError> {
    CALLBACK_FAILED.with(|failed| failed.set(false));
    RETIRED_SURFACES.with(|retired| retired.borrow_mut().clear());
    // Force creation of the thread message queue before reporting readiness.
    let mut bootstrap = MSG::default();
    unsafe {
        let _ = PeekMessageW(&mut bootstrap, None, WM_USER, WM_USER, PM_NOREMOVE);
    };
    let resources = WorkerResources::register(generation)?;
    let token = WorkerToken {
        thread_id: unsafe { GetCurrentThreadId() },
        generation,
    };
    context.availability.store(true, Ordering::Release);
    ready
        .send(Ok(token))
        .map_err(|_| WindowsConsentError::WorkerUnavailable)?;

    let mut active = ActiveSurfaceGuard::default();
    let loop_result = catch_unwind(AssertUnwindSafe(|| {
        run_worker_loop(&resources, generation, context, active.slot())
    }));
    context.availability.store(false, Ordering::Release);
    context.shutdown.store(true, Ordering::Release);
    close_active_surface(active.slot(), ConsentSurfaceDecision::Dismissed);
    dismiss_pending_requests(context);
    drop(active);
    drain_retired_surfaces();
    drop(resources);
    match loop_result {
        Ok(outcome) => outcome,
        Err(_) => Err(WindowsConsentError::WorkerUnavailable),
    }
}

fn dismiss_pending_requests(context: &WorkerContext) {
    let pending = context
        .show
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .drain(..)
        .collect::<Vec<_>>();
    for request in pending {
        let request_generation = request.generation();
        context.close.release(request_generation);
        request.finish_destroyed(ConsentSurfaceDecision::Dismissed);
    }
}

fn run_worker_loop(
    resources: &WorkerResources,
    worker_generation: usize,
    context: &WorkerContext,
    active: &mut Option<(HWND, u64)>,
) -> Result<(), WindowsConsentError> {
    let mut message = MSG::default();
    loop {
        // Sent messages can run WndProc inside MsgWait/PeekMessage without
        // yielding a queued MSG. Reap those destroyed surfaces before any
        // newly-admitted request attempts to attach its generation.
        drain_retired_surfaces();
        if active
            .as_ref()
            .is_some_and(|(window, _)| !unsafe { IsWindow(Some(*window)).as_bool() })
        {
            *active = None;
        }
        if context.shutdown.load(Ordering::Acquire) || callback_failed() {
            return if callback_failed() {
                Err(WindowsConsentError::WorkerUnavailable)
            } else {
                Ok(())
            };
        }
        if let Some((window, prompt_generation)) = *active {
            if context.close.is_requested(prompt_generation) {
                set_surface_decision(window, ConsentSurfaceDecision::Dismissed);
                destroy_surface(window)?;
                *active = None;
                drain_retired_surfaces();
                continue;
            }
        }

        if active.is_none()
            && unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() }
        {
            if message.message == WM_QUIT {
                return Err(WindowsConsentError::WorkerUnavailable);
            }
            // No surface exists, so every queued window/control message is
            // stale. Drain the queue before creating the next generation;
            // sent messages dispatched inside PeekMessage carry no authority.
            continue;
        }

        let request = {
            context
                .show
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .pop_front()
        };
        if let Some(request) = request {
            let request_generation = request.generation();
            if context.shutdown.load(Ordering::Acquire)
                || context.close.is_requested(request_generation)
            {
                context.close.release(request_generation);
                request.finish_destroyed(ConsentSurfaceDecision::Dismissed);
                continue;
            }
            if active
                .as_ref()
                .is_some_and(|(window, _)| unsafe { IsWindow(Some(*window)).as_bool() })
            {
                context.close.release(request_generation);
                request.finish_destroyed(ConsentSurfaceDecision::Dismissed);
                context.availability.store(false, Ordering::Release);
                return Err(WindowsConsentError::WorkerUnavailable);
            }
            *active = None;
            match open_surface(
                resources,
                request,
                Arc::clone(&context.close),
                Arc::clone(&context.reclaim),
                Arc::clone(&context.availability),
            ) {
                Ok(window) => *active = Some((window, request_generation)),
                Err(error) => {
                    context.availability.store(false, Ordering::Release);
                    drain_retired_surfaces();
                    return Err(error);
                }
            }
            continue;
        }

        let wait = unsafe {
            MsgWaitForMultipleObjectsEx(
                Some(&[context.wake.0]),
                INFINITE,
                QS_ALLINPUT,
                MWMO_INPUTAVAILABLE,
            )
        };
        if wait == WAIT_FAILED {
            return Err(last_native_error("wait for consent command"));
        }
        if wait == WAIT_OBJECT_0 {
            continue;
        }
        if wait.0 != WAIT_OBJECT_0.0 + 1 {
            return Err(WindowsConsentError::WorkerUnavailable);
        }
        if !unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() } {
            continue;
        }
        if message.message == WM_QUIT {
            return Err(WindowsConsentError::WorkerUnavailable);
        }
        if message.message == WM_MRD_WAKE_CONSENT {
            // A message is only the fallback for an unexpectedly unavailable
            // event. All actual command authority lives in atomics/queues.
            let _is_current_worker = message.wParam.0 == worker_generation;
            continue;
        }

        let dispatch_authority = active
            .as_ref()
            .and_then(|(window, _)| surface_authority(*window));
        if let Some((window, _)) = *active {
            if is_keyboard_decision_message(message.message, message.wParam.0) {
                let _dispatch = dispatch_authority.map(|authority| {
                    PromptAuthorityGuard::enter(AuthorityKind::Dispatch, authority)
                });
                unsafe {
                    DispatchMessageW(&message);
                }
                drain_retired_surfaces();
                if !unsafe { IsWindow(Some(window)).as_bool() } {
                    *active = None;
                }
                continue;
            }
            let _dispatch = dispatch_authority
                .map(|authority| PromptAuthorityGuard::enter(AuthorityKind::Dispatch, authority));
            if unsafe { IsDialogMessageW(window, &message).as_bool() } {
                drain_retired_surfaces();
                if !unsafe { IsWindow(Some(window)).as_bool() } {
                    *active = None;
                }
                continue;
            }
        }
        let _dispatch = dispatch_authority
            .map(|authority| PromptAuthorityGuard::enter(AuthorityKind::Dispatch, authority));
        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        drain_retired_surfaces();
        if let Some((window, _)) = *active {
            if !unsafe { IsWindow(Some(window)).as_bool() } {
                *active = None;
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InputMessageOrigin {
    Hardware,
    Injected,
    System,
    Unavailable,
}

fn current_input_is_hardware_for(message: u32) -> bool {
    // Windows intentionally defines IMO_HARDWARE to include UIAccess=true
    // applications that inject into a queue. The attended-agent boundary
    // treats such UIAccess processes as OS-trusted; ordinary SendInput is
    // IMO_INJECTED and posted/sent messages have no accepted device class.
    let mut source = INPUT_MESSAGE_SOURCE::default();
    if unsafe { GetCurrentInputMessageSource(&mut source) }.is_err()
        || source.originId != IMO_HARDWARE
    {
        return false;
    }
    match message {
        WM_KEYDOWN | WM_KEYUP => source.deviceType == IMDT_KEYBOARD,
        WM_LBUTTONDOWN | WM_LBUTTONUP => matches!(
            source.deviceType,
            IMDT_MOUSE | IMDT_TOUCH | IMDT_TOUCHPAD | IMDT_PEN
        ),
        WM_SYSCOMMAND => matches!(
            source.deviceType,
            IMDT_KEYBOARD | IMDT_MOUSE | IMDT_TOUCH | IMDT_TOUCHPAD | IMDT_PEN
        ),
        _ => false,
    }
}

fn keyboard_decision(
    message: u32,
    key: usize,
    origin: InputMessageOrigin,
) -> Option<ConsentSurfaceDecision> {
    if origin != InputMessageOrigin::Hardware {
        return None;
    }
    match (message, key) {
        (WM_KEYDOWN, VK_ESCAPE_CODE) => Some(ConsentSurfaceDecision::Dismissed),
        (WM_KEYDOWN, VK_RETURN_CODE) => Some(ConsentSurfaceDecision::Denied),
        _ => None,
    }
}

fn is_keyboard_decision_message(message: u32, key: usize) -> bool {
    matches!(message, WM_KEYDOWN | WM_KEYUP) && matches!(key, VK_ESCAPE_CODE | VK_RETURN_CODE)
}

struct ScopeCheckbox {
    scope: PermissionScope,
    control: ControlBinding,
}

struct SurfaceState {
    attached: Arc<AtomicBool>,
    reclaim: Arc<AtomicUsize>,
    generation: u64,
    request: Option<ConsentSurfaceRequest>,
    close: Arc<CloseLatch>,
    availability: Arc<AtomicBool>,
    window: HWND,
    binding: CommandBinding,
    checkboxes: Vec<ScopeCheckbox>,
    decision: Option<ConsentSurfaceDecision>,
    #[cfg(test)]
    initial_focus_verified: bool,
}

impl Drop for SurfaceState {
    fn drop(&mut self) {
        let raw = std::ptr::from_mut(self) as usize;
        let _ = self
            .reclaim
            .compare_exchange(raw, 0, Ordering::AcqRel, Ordering::Acquire);
        if let Some(request) = self.request.take() {
            self.close.release(self.generation);
            request.finish_destroyed(ConsentSurfaceDecision::Dismissed);
        }
    }
}

struct SurfaceWindowGuard {
    window: HWND,
    armed: bool,
}

impl SurfaceWindowGuard {
    fn new(window: HWND) -> Self {
        Self {
            window,
            armed: true,
        }
    }

    fn dismiss(&mut self) -> Result<(), WindowsConsentError> {
        set_surface_decision(self.window, ConsentSurfaceDecision::Dismissed);
        destroy_surface(self.window)?;
        self.armed = false;
        drain_retired_surfaces();
        Ok(())
    }

    fn release(mut self) -> HWND {
        self.armed = false;
        self.window
    }
}

impl Drop for SurfaceWindowGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        set_surface_decision(self.window, ConsentSurfaceDecision::Dismissed);
        if destroy_surface(self.window).is_err() {
            // Preserve fail-closed health and force worker teardown. Windows
            // then reclaims any same-thread HWND that resisted explicit close.
            let _ = callback_fail_stop(self.window);
        }
        drain_retired_surfaces();
    }
}

fn open_surface(
    resources: &WorkerResources,
    request: ConsentSurfaceRequest,
    close: Arc<CloseLatch>,
    reclaim: Arc<AtomicUsize>,
    availability: Arc<AtomicBool>,
) -> Result<HWND, WindowsConsentError> {
    let generation = request.generation();
    let session_line = prefixed_utf16("Session: ", request.model().session_id_utf16());
    let device_line = prefixed_utf16("Device: ", request.model().device_id_utf16());
    let fingerprint_lines = fingerprint_utf16(request.model().peer_fingerprint());
    let scope_models = request
        .model()
        .scopes()
        .iter()
        .map(|scope| (scope.scope(), nul_terminated(scope.label().encode_utf16())))
        .collect::<Vec<_>>();
    let surface_authority = PromptAuthority {
        generation,
        token: next_surface_authority()?,
    };
    let deny_authority = PromptAuthority {
        generation,
        token: next_surface_authority()?,
    };
    let allow_authority = PromptAuthority {
        generation,
        token: next_surface_authority()?,
    };
    let attached = Arc::new(AtomicBool::new(false));
    let state = Box::new(SurfaceState {
        attached: Arc::clone(&attached),
        reclaim,
        generation,
        request: Some(request),
        close,
        availability,
        window: HWND::default(),
        binding: CommandBinding {
            surface: surface_authority,
            deny: ButtonBinding {
                control: ControlBinding {
                    authority: deny_authority,
                    surface: surface_authority,
                    parent_window: 0,
                    control_id: DENY_CONTROL_ID,
                    control_window: 0,
                },
                action: ButtonAction::Deny,
            },
            allow: ButtonBinding {
                control: ControlBinding {
                    authority: allow_authority,
                    surface: surface_authority,
                    parent_window: 0,
                    control_id: ALLOW_CONTROL_ID,
                    control_window: 0,
                },
                action: ButtonAction::Allow,
            },
        },
        checkboxes: Vec::with_capacity(scope_models.len()),
        decision: Some(ConsentSurfaceDecision::Dismissed),
        #[cfg(test)]
        initial_focus_verified: false,
    });
    let raw = Box::into_raw(state);
    let height = surface_height(scope_models.len());
    let window_result = {
        let _creation_authority =
            PromptAuthorityGuard::enter(AuthorityKind::Destroy, surface_authority);
        let _create_state = CreateStateGuard::enter(raw);
        unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                PCWSTR(resources.class_name.as_ptr()),
                w!("Remote desktop consent"),
                WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_CLIPCHILDREN,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                SURFACE_WIDTH,
                height,
                None,
                None,
                Some(resources.instance),
                Some(raw.cast()),
            )
        }
    };
    let window = match window_result {
        Ok(window) => window,
        Err(error) => {
            if !attached.load(Ordering::Acquire) {
                unsafe { drop(Box::from_raw(raw)) };
            }
            return Err(native_error("create consent window", error));
        }
    };
    // From here through successful return, every error and unwind path owns a
    // deterministic attempt to destroy the newly-created top-level window.
    let mut window_guard = SurfaceWindowGuard::new(window);

    let result = create_surface_controls(
        raw,
        window,
        &session_line,
        &device_line,
        &fingerprint_lines,
        &scope_models,
        height,
    );
    if let Err(error) = result {
        unsafe { (*raw).availability.store(false, Ordering::Release) };
        window_guard.dismiss()?;
        return Err(error);
    }
    unsafe {
        let _ = ShowWindow(window, SW_SHOW);
    }
    if !unsafe { UpdateWindow(window).as_bool() } {
        let error = last_native_error("update consent window");
        unsafe { (*raw).availability.store(false, Ordering::Release) };
        window_guard.dismiss()?;
        return Err(error);
    }
    let deny = HWND(unsafe { (*raw).binding.deny.control.control_window } as *mut c_void);
    let _ = unsafe { SetForegroundWindow(window) };
    let _ = unsafe { SetActiveWindow(window) };
    let _ = unsafe { SetFocus(Some(deny)) };
    if unsafe { GetFocus() } != deny {
        let error = last_native_error("focus deny control");
        unsafe { (*raw).availability.store(false, Ordering::Release) };
        window_guard.dismiss()?;
        return Err(error);
    }
    #[cfg(test)]
    unsafe {
        (*raw).initial_focus_verified = true;
    }
    Ok(window_guard.release())
}

fn surface_height(scope_count: usize) -> i32 {
    let rows = scope_count.div_ceil(SCOPE_COLUMNS);
    276_i32.saturating_add(
        i32::try_from(rows)
            .unwrap_or(i32::MAX)
            .saturating_mul(SCOPE_ROW_HEIGHT),
    )
}

fn create_surface_controls(
    raw: *mut SurfaceState,
    window: HWND,
    session_line: &[u16],
    device_line: &[u16],
    fingerprint_lines: &[Vec<u16>; 2],
    scopes: &[(PermissionScope, Vec<u16>)],
    height: i32,
) -> Result<(), WindowsConsentError> {
    let font = unsafe { GetStockObject(DEFAULT_GUI_FONT) };
    let heading = create_control(
        w!("STATIC"),
        &nul_terminated("A remote device is requesting access.".encode_utf16()),
        WS_CHILD | WS_VISIBLE,
        24,
        20,
        CONTENT_WIDTH,
        24,
        window,
        None,
    )?;
    apply_font(heading, font.0 as usize);
    for (line, y) in [(session_line, 54), (device_line, 80)] {
        let control = create_control(
            w!("STATIC"),
            line,
            WS_CHILD | WS_VISIBLE,
            24,
            y,
            CONTENT_WIDTH,
            22,
            window,
            None,
        )?;
        apply_font(control, font.0 as usize);
    }
    for (line, y) in fingerprint_lines.iter().zip([106, 132]) {
        let control = create_control(
            w!("STATIC"),
            line,
            WS_CHILD | WS_VISIBLE,
            24,
            y,
            CONTENT_WIDTH,
            22,
            window,
            None,
        )?;
        apply_font(control, font.0 as usize);
    }

    let mut checkbox_states = Vec::with_capacity(scopes.len());
    let surface_authority = unsafe { (*raw).binding.surface };
    for (index, (scope, label)) in scopes.iter().enumerate() {
        let control_id = FIRST_SCOPE_CONTROL_ID
            .checked_add(u16::try_from(index).map_err(|_| WindowsConsentError::WorkerUnavailable)?)
            .ok_or(WindowsConsentError::WorkerUnavailable)?;
        let checkbox = create_control(
            w!("BUTTON"),
            label,
            WINDOW_STYLE(
                WS_CHILD.0
                    | WS_VISIBLE.0
                    | WS_TABSTOP.0
                    | u32::try_from(BS_AUTOCHECKBOX).unwrap_or(0),
            ),
            40 + i32::try_from(index % SCOPE_COLUMNS)
                .unwrap_or(i32::MAX)
                .saturating_mul(SCOPE_COLUMN_WIDTH),
            SCOPE_BASE_Y
                + i32::try_from(index / SCOPE_COLUMNS)
                    .unwrap_or(i32::MAX)
                    .saturating_mul(SCOPE_ROW_HEIGHT),
            330,
            24,
            window,
            Some(control_id),
        )?;
        apply_font(checkbox, font.0 as usize);
        checkbox_states.push(ScopeCheckbox {
            scope: *scope,
            control: ControlBinding {
                authority: PromptAuthority {
                    generation: surface_authority.generation,
                    token: next_surface_authority()?,
                },
                surface: surface_authority,
                parent_window: window.0 as isize,
                control_id,
                control_window: checkbox.0 as isize,
            },
        });
    }
    let button_y = height.saturating_sub(76);
    let deny = create_control(
        w!("BUTTON"),
        &nul_terminated("Deny".encode_utf16()),
        WINDOW_STYLE(
            WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | u32::try_from(BS_DEFPUSHBUTTON).unwrap_or(0),
        ),
        520,
        button_y,
        100,
        32,
        window,
        Some(DENY_CONTROL_ID),
    )?;
    let allow = create_control(
        w!("BUTTON"),
        &nul_terminated("Allow".encode_utf16()),
        WINDOW_STYLE(
            WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | u32::try_from(BS_PUSHBUTTON).unwrap_or(0),
        ),
        636,
        button_y,
        100,
        32,
        window,
        Some(ALLOW_CONTROL_ID),
    )?;
    apply_font(deny, font.0 as usize);
    apply_font(allow, font.0 as usize);
    unsafe {
        (*raw).binding.deny.control.parent_window = window.0 as isize;
        (*raw).binding.deny.control.control_window = deny.0 as isize;
        (*raw).binding.allow.control.parent_window = window.0 as isize;
        (*raw).binding.allow.control.control_window = allow.0 as isize;
        (*raw).checkboxes = checkbox_states;
    }
    install_control_subclass(unsafe { &(*raw).binding.deny.control })?;
    install_control_subclass(unsafe { &(*raw).binding.allow.control })?;
    for checkbox in unsafe { &(*raw).checkboxes } {
        install_control_subclass(&checkbox.control)?;
    }
    Ok(())
}

fn install_control_subclass(binding: &ControlBinding) -> Result<(), WindowsConsentError> {
    let window = HWND(binding.control_window as *mut c_void);
    let installed = unsafe {
        SetWindowSubclass(
            window,
            Some(consent_control_proc),
            binding.authority.token,
            std::ptr::from_ref(binding) as usize,
        )
    };
    if installed.as_bool() {
        Ok(())
    } else {
        Err(last_native_error("subclass consent button"))
    }
}

#[allow(clippy::too_many_arguments)]
fn create_control(
    class: PCWSTR,
    text: &[u16],
    style: WINDOW_STYLE,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    parent: HWND,
    id: Option<u16>,
) -> Result<HWND, WindowsConsentError> {
    let menu = id.map(|id| HMENU(usize::from(id) as *mut c_void));
    unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class,
            PCWSTR(text.as_ptr()),
            style,
            x,
            y,
            width,
            height,
            Some(parent),
            menu,
            None,
            None,
        )
    }
    .map_err(|error| native_error("create consent control", error))
}

fn apply_font(window: HWND, font: usize) {
    unsafe {
        SendMessageW(window, WM_SETFONT, Some(WPARAM(font)), Some(LPARAM(1)));
    }
}

fn prefixed_utf16(prefix: &str, value: &[u16]) -> Vec<u16> {
    let mut result = prefix.encode_utf16().collect::<Vec<_>>();
    result.extend_from_slice(value);
    result.push(0);
    result
}

fn fingerprint_utf16(fingerprint: &[u8; 32]) -> [Vec<u16>; 2] {
    std::array::from_fn(|line| {
        let mut text = String::with_capacity(20 + 16 * 3);
        use std::fmt::Write as _;
        let _ = write!(&mut text, "Key SHA-256 ({}/2): ", line + 1);
        for (index, byte) in fingerprint[line * 16..(line + 1) * 16].iter().enumerate() {
            if index != 0 {
                text.push(':');
            }
            let _ = write!(&mut text, "{byte:02X}");
        }
        nul_terminated(text.encode_utf16())
    })
}

fn nul_terminated(units: impl IntoIterator<Item = u16>) -> Vec<u16> {
    units.into_iter().chain(std::iter::once(0)).collect()
}

#[derive(Clone, Copy)]
enum AuthorityKind {
    Control,
    Dispatch,
    Keyboard,
    Close,
    Destroy,
}

struct PromptAuthorityGuard {
    kind: AuthorityKind,
    previous: Option<PromptAuthority>,
}

impl PromptAuthorityGuard {
    fn enter(kind: AuthorityKind, authority: PromptAuthority) -> Self {
        let previous = match kind {
            AuthorityKind::Control => TRUSTED_CONTROL.with(|slot| slot.replace(Some(authority))),
            AuthorityKind::Dispatch => TRUSTED_DISPATCH.with(|slot| slot.replace(Some(authority))),
            AuthorityKind::Keyboard => TRUSTED_KEYBOARD.with(|slot| slot.replace(Some(authority))),
            AuthorityKind::Close => TRUSTED_CLOSE.with(|slot| slot.replace(Some(authority))),
            AuthorityKind::Destroy => TRUSTED_DESTROY.with(|slot| slot.replace(Some(authority))),
        };
        Self { kind, previous }
    }
}

impl Drop for PromptAuthorityGuard {
    fn drop(&mut self) {
        match self.kind {
            AuthorityKind::Control => TRUSTED_CONTROL.with(|slot| slot.set(self.previous)),
            AuthorityKind::Dispatch => TRUSTED_DISPATCH.with(|slot| slot.set(self.previous)),
            AuthorityKind::Keyboard => TRUSTED_KEYBOARD.with(|slot| slot.set(self.previous)),
            AuthorityKind::Close => TRUSTED_CLOSE.with(|slot| slot.set(self.previous)),
            AuthorityKind::Destroy => TRUSTED_DESTROY.with(|slot| slot.set(self.previous)),
        }
    }
}

struct CreateStateGuard(usize);

impl CreateStateGuard {
    fn enter(raw: *mut SurfaceState) -> Self {
        Self(TRUSTED_CREATE_STATE.with(|slot| slot.replace(raw as usize)))
    }
}

impl Drop for CreateStateGuard {
    fn drop(&mut self) {
        TRUSTED_CREATE_STATE.with(|slot| slot.set(self.0));
    }
}

unsafe extern "system" fn consent_control_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    subclass_id: usize,
    reference_data: usize,
) -> LRESULT {
    match catch_unwind(AssertUnwindSafe(|| unsafe {
        consent_control_proc_inner(window, message, wparam, lparam, subclass_id, reference_data)
    })) {
        Ok(result) => result,
        Err(_) => callback_fail_stop_for_child(window),
    }
}

#[allow(clippy::too_many_arguments)]
unsafe fn consent_control_proc_inner(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    subclass_id: usize,
    reference_data: usize,
) -> LRESULT {
    let binding = reference_data as *const ControlBinding;
    if binding.is_null() {
        return unsafe { DefSubclassProc(window, message, wparam, lparam) };
    }
    let binding = unsafe { &*binding };
    if subclass_id != binding.authority.token || window.0 as isize != binding.control_window {
        return callback_fail_stop(HWND(binding.parent_window as *mut c_void));
    }
    if message == WM_NCDESTROY {
        let removed = unsafe {
            RemoveWindowSubclass(window, Some(consent_control_proc), binding.authority.token)
        };
        let result = unsafe { DefSubclassProc(window, message, wparam, lparam) };
        if !removed.as_bool() {
            let _ = callback_fail_stop(HWND(binding.parent_window as *mut c_void));
        }
        return result;
    }
    #[cfg(test)]
    if message == WM_MRD_QUERY_CONTROL_AUTHORITY {
        return match wparam.0 {
            0 => LRESULT(binding.authority.generation as isize),
            1 => LRESULT(binding.authority.token as isize),
            _ => LRESULT(0),
        };
    }
    #[cfg(test)]
    if message == WM_MRD_TEST_CONTROL_ACTION {
        if !explicit_authority_is_current(binding.authority, wparam.0 as u64, lparam.0 as usize) {
            return LRESULT(0);
        }
        let _authority = PromptAuthorityGuard::enter(AuthorityKind::Control, binding.authority);
        if binding.control_id >= FIRST_SCOPE_CONTROL_ID {
            let checked = unsafe { DefSubclassProc(window, BM_GETCHECK, WPARAM(0), LPARAM(0)).0 };
            let next = usize::from(checked == 0);
            let _ = unsafe { DefSubclassProc(window, BM_SETCHECK, WPARAM(next), LPARAM(0)) };
        }
        let command = usize::from(binding.control_id) | ((BN_CLICKED as usize) << 16);
        unsafe {
            SendMessageW(
                HWND(binding.parent_window as *mut c_void),
                WM_COMMAND,
                Some(WPARAM(command)),
                Some(LPARAM(window.0 as isize)),
            );
        }
        return LRESULT(1);
    }
    if is_keyboard_decision_message(message, wparam.0) {
        if TRUSTED_DISPATCH.with(Cell::get) == Some(binding.surface) {
            let origin = if current_input_is_hardware_for(message) {
                InputMessageOrigin::Hardware
            } else {
                InputMessageOrigin::Unavailable
            };
            if let Some(decision) = keyboard_decision(message, wparam.0, origin) {
                let code = match decision {
                    ConsentSurfaceDecision::Denied => 1,
                    ConsentSurfaceDecision::Dismissed | ConsentSurfaceDecision::Approved(_) => 0,
                };
                let _keyboard =
                    PromptAuthorityGuard::enter(AuthorityKind::Keyboard, binding.surface);
                unsafe {
                    SendMessageW(
                        HWND(binding.parent_window as *mut c_void),
                        WM_MRD_KEYBOARD_DECISION,
                        Some(WPARAM(code)),
                        Some(LPARAM(window.0 as isize)),
                    );
                }
            }
        }
        return LRESULT(0);
    }
    if message == BM_SETCHECK {
        return if TRUSTED_CONTROL.with(Cell::get) == Some(binding.authority) {
            unsafe { DefSubclassProc(window, message, wparam, lparam) }
        } else {
            LRESULT(0)
        };
    }
    if !control_message_is_authorized(
        binding,
        TRUSTED_DISPATCH.with(Cell::get),
        message,
        wparam.0,
        if current_input_is_hardware_for(message) {
            InputMessageOrigin::Hardware
        } else {
            InputMessageOrigin::Unavailable
        },
    ) {
        return LRESULT(0);
    }
    let is_activation =
        message == WM_LBUTTONUP || (message == WM_KEYUP && wparam.0 == VK_SPACE_CODE);
    if is_activation {
        let _authority = PromptAuthorityGuard::enter(AuthorityKind::Control, binding.authority);
        unsafe { DefSubclassProc(window, message, wparam, lparam) }
    } else {
        unsafe { DefSubclassProc(window, message, wparam, lparam) }
    }
}

fn control_message_is_authorized(
    binding: &ControlBinding,
    dispatch: Option<PromptAuthority>,
    message: u32,
    wparam: usize,
    origin: InputMessageOrigin,
) -> bool {
    let is_pointer_input = matches!(message, WM_LBUTTONDOWN | WM_LBUTTONUP);
    let is_space_input = matches!(message, WM_KEYDOWN | WM_KEYUP) && wparam == VK_SPACE_CODE;
    if matches!(message, BM_CLICK | BM_SETCHECK) {
        return false;
    }
    if is_pointer_input || is_space_input {
        return dispatch == Some(binding.surface) && origin == InputMessageOrigin::Hardware;
    }
    true
}

fn explicit_authority_is_current(
    authority: PromptAuthority,
    generation: u64,
    token: usize,
) -> bool {
    authority == PromptAuthority { generation, token }
}

fn callback_fail_stop_for_child(window: HWND) -> LRESULT {
    let parent = unsafe { GetParent(window) }.unwrap_or_default();
    callback_fail_stop(parent)
}

unsafe extern "system" fn consent_window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match catch_unwind(AssertUnwindSafe(|| unsafe {
        consent_window_proc_inner(window, message, wparam, lparam)
    })) {
        Ok(result) => result,
        Err(_) => callback_fail_stop(window),
    }
}

unsafe fn consent_window_proc_inner(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_NCCREATE {
        // lpCreateParams is an untrusted message pointer. The worker instead
        // supplies the exact allocation through a same-thread scoped token
        // around CreateWindowExW, so a synthetic WM_NCCREATE is harmless.
        let raw = TRUSTED_CREATE_STATE.with(Cell::get) as *mut SurfaceState;
        if raw.is_null() {
            return LRESULT(0);
        }
        if TRUSTED_DESTROY.with(Cell::get) != Some(unsafe { (*raw).binding.surface }) {
            return LRESULT(0);
        }
        unsafe {
            if (*raw)
                .reclaim
                .compare_exchange(0, raw as usize, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
            {
                SetLastError(ERROR_BUSY);
                return LRESULT(0);
            }
            (*raw).window = window;
            SetLastError(WIN32_ERROR(0));
            let previous = SetWindowLongPtrW(window, GWLP_USERDATA, raw as isize);
            let status = GetLastError();
            if previous == 0 && status != WIN32_ERROR(0) {
                return LRESULT(0);
            }
            (*raw).attached.store(true, Ordering::Release);
        }
        return LRESULT(1);
    }
    let raw = unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) } as *mut SurfaceState;
    if raw.is_null() {
        return unsafe { DefWindowProcW(window, message, wparam, lparam) };
    }
    match message {
        #[cfg(test)]
        WM_MRD_QUERY_INITIAL_FOCUS => {
            LRESULT(isize::from(unsafe { (*raw).initial_focus_verified }))
        }
        #[cfg(test)]
        WM_MRD_QUERY_SURFACE_AUTHORITY => {
            let authority = unsafe { (*raw).binding.surface };
            match wparam.0 {
                0 => LRESULT(authority.generation as isize),
                1 => LRESULT(authority.token as isize),
                _ => LRESULT(0),
            }
        }
        #[cfg(test)]
        WM_MRD_TEST_SURFACE_DISMISS => {
            let authority = unsafe { (*raw).binding.surface };
            if explicit_authority_is_current(authority, wparam.0 as u64, lparam.0 as usize) {
                close_surface_raw(raw, ConsentSurfaceDecision::Dismissed);
            }
            LRESULT(0)
        }
        WM_MRD_KEYBOARD_DECISION => {
            let binding = unsafe { &(*raw).binding };
            if TRUSTED_KEYBOARD.with(Cell::get) != Some(binding.surface)
                || !is_current_control_window(unsafe { &*raw }, lparam.0)
            {
                return LRESULT(0);
            }
            let decision = if wparam.0 == 1 {
                ConsentSurfaceDecision::Denied
            } else {
                ConsentSurfaceDecision::Dismissed
            };
            close_surface_raw(raw, decision);
            LRESULT(0)
        }
        message @ (WM_KEYDOWN | WM_KEYUP) if is_keyboard_decision_message(message, wparam.0) => {
            let binding = unsafe { &(*raw).binding };
            if TRUSTED_DISPATCH.with(Cell::get) != Some(binding.surface) {
                return LRESULT(0);
            }
            let origin = if current_input_is_hardware_for(message) {
                InputMessageOrigin::Hardware
            } else {
                InputMessageOrigin::Unavailable
            };
            if let Some(decision) = keyboard_decision(message, wparam.0, origin) {
                close_surface_raw(raw, decision);
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            let command_id = (wparam.0 & 0xffff) as u16;
            let notification = ((wparam.0 >> 16) & 0xffff) as u16;
            let source = lparam.0;
            if is_exact_scope_toggle(
                unsafe { &(*raw).checkboxes },
                command_id,
                notification,
                source,
            ) {
                return LRESULT(0);
            }
            let binding = unsafe { &(*raw).binding };
            match classify_command(
                binding,
                TRUSTED_CONTROL.with(Cell::get),
                command_id,
                notification,
                source,
            ) {
                NativeCommand::Deny(_) => {
                    close_surface_raw(raw, ConsentSurfaceDecision::Denied);
                }
                NativeCommand::Allow(_) => {
                    let decision = selected_scope_decision(raw);
                    close_surface_raw(raw, decision);
                }
                NativeCommand::Dismiss(_) => {
                    close_surface_raw(raw, ConsentSurfaceDecision::Dismissed);
                }
                NativeCommand::Ignore => {}
            }
            LRESULT(0)
        }
        WM_SYSCOMMAND if (wparam.0 & 0xfff0) == SC_CLOSE as usize => {
            let binding = unsafe { &(*raw).binding };
            if !system_close_is_authorized(
                binding,
                TRUSTED_DISPATCH.with(Cell::get),
                if current_input_is_hardware_for(message) {
                    InputMessageOrigin::Hardware
                } else {
                    InputMessageOrigin::Unavailable
                },
            ) {
                return LRESULT(0);
            }
            let authority = binding.surface;
            let _authority = PromptAuthorityGuard::enter(AuthorityKind::Close, authority);
            unsafe { DefWindowProcW(window, message, wparam, lparam) }
        }
        WM_CLOSE => {
            let binding = unsafe { &(*raw).binding };
            if matches!(
                classify_close(binding, TRUSTED_CLOSE.with(Cell::get)),
                NativeCommand::Dismiss(_)
            ) {
                close_surface_raw(raw, ConsentSurfaceDecision::Dismissed);
            }
            LRESULT(0)
        }
        WM_NCDESTROY => {
            let binding = unsafe { &(*raw).binding };
            if !destroy_is_authorized(binding, TRUSTED_DESTROY.with(Cell::get)) {
                return callback_fail_stop(window);
            }
            let result = unsafe { DefWindowProcW(window, message, wparam, lparam) };
            unsafe {
                SetWindowLongPtrW(window, GWLP_USERDATA, 0);
                retire_destroyed(raw);
            }
            result
        }
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}

fn is_exact_scope_toggle(
    checkboxes: &[ScopeCheckbox],
    command_id: u16,
    notification: u16,
    source_window: isize,
) -> bool {
    notification == BN_CLICKED as u16
        && checkboxes.iter().any(|checkbox| {
            checkbox.control.control_id == command_id
                && checkbox.control.control_window == source_window
        })
}

fn system_close_is_authorized(
    binding: &CommandBinding,
    dispatch: Option<PromptAuthority>,
    origin: InputMessageOrigin,
) -> bool {
    dispatch == Some(binding.surface) && origin == InputMessageOrigin::Hardware
}

fn selected_scope_decision(raw: *mut SurfaceState) -> ConsentSurfaceDecision {
    let state = unsafe { &*raw };
    let mut scopes = PermissionScopes::new();
    for checkbox in &state.checkboxes {
        let checked = unsafe {
            SendMessageW(
                HWND(checkbox.control.control_window as *mut c_void),
                BM_GETCHECK,
                Some(WPARAM(0)),
                Some(LPARAM(0)),
            )
            .0
        };
        match checked {
            0 => {}
            1 => {
                scopes.insert(checkbox.scope);
            }
            _ => return ConsentSurfaceDecision::Dismissed,
        }
    }
    if scopes.is_empty() {
        ConsentSurfaceDecision::Dismissed
    } else {
        ConsentSurfaceDecision::Approved(scopes)
    }
}

fn is_current_control_window(state: &SurfaceState, window: isize) -> bool {
    state.binding.deny.control.control_window == window
        || state.binding.allow.control.control_window == window
        || state
            .checkboxes
            .iter()
            .any(|checkbox| checkbox.control.control_window == window)
}

fn close_surface_raw(raw: *mut SurfaceState, decision: ConsentSurfaceDecision) {
    let window = unsafe {
        (*raw).decision = Some(decision);
        (*raw).window
    };
    if destroy_surface(window).is_err() {
        unsafe { (*raw).availability.store(false, Ordering::Release) };
        let _ = callback_fail_stop(window);
    }
}

unsafe fn retire_destroyed(raw: *mut SurfaceState) {
    RETIRED_SURFACES.with(|retired| retired.borrow_mut().push(raw));
}

unsafe fn complete_retired_surface(raw: *mut SurfaceState) {
    let state = unsafe { &mut *raw };
    let decision = if state.close.is_requested(state.generation) {
        ConsentSurfaceDecision::Dismissed
    } else {
        state
            .decision
            .take()
            .unwrap_or(ConsentSurfaceDecision::Dismissed)
    };
    state.close.release(state.generation);
    if let Some(request) = state.request.take() {
        request.finish_destroyed(decision);
    }
}

fn surface_authority(window: HWND) -> Option<PromptAuthority> {
    let raw = unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) } as *mut SurfaceState;
    (!raw.is_null()).then(|| unsafe { (*raw).binding.surface })
}

fn set_surface_decision(window: HWND, decision: ConsentSurfaceDecision) {
    let raw = unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) } as *mut SurfaceState;
    if !raw.is_null() {
        unsafe { (*raw).decision = Some(decision) };
    }
}

fn destroy_surface(window: HWND) -> Result<(), WindowsConsentError> {
    if !unsafe { IsWindow(Some(window)).as_bool() } {
        return Ok(());
    }
    let raw = unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) } as *mut SurfaceState;
    if raw.is_null() {
        return Err(WindowsConsentError::WorkerUnavailable);
    }
    let authority = unsafe { (*raw).binding.surface };
    let _authority = PromptAuthorityGuard::enter(AuthorityKind::Destroy, authority);
    unsafe { DestroyWindow(window) }.map_err(|error| native_error("destroy consent window", error))
}

fn close_active_surface(active: &mut Option<(HWND, u64)>, decision: ConsentSurfaceDecision) {
    let window = active.as_ref().map(|(window, _)| *window);
    let destroyed = retain_owner_until_destroyed(active, |(window, _)| {
        set_surface_decision(window, decision);
        destroy_surface(window).is_ok()
    });
    if !destroyed {
        // Keep logical ownership in the guard so its Drop performs one final
        // close attempt; the supervisor reclaims it after joining the worker.
        if let Some(window) = window {
            let _ = callback_fail_stop(window);
        }
    }
    drain_retired_surfaces();
}

fn retain_owner_until_destroyed<T: Copy>(
    owner: &mut Option<T>,
    destroy: impl FnOnce(T) -> bool,
) -> bool {
    let Some(value) = *owner else {
        return true;
    };
    if destroy(value) {
        *owner = None;
        true
    } else {
        false
    }
}

fn drain_retired_surfaces() {
    RETIRED_SURFACES.with(|retired| {
        let pointers = retired.borrow_mut().drain(..).collect::<Vec<_>>();
        for raw in pointers {
            unsafe { purge_surface_messages(&*raw) };
            unsafe { complete_retired_surface(raw) };
            unsafe { drop(Box::from_raw(raw)) };
        }
    });
}

fn purge_surface_messages(state: &SurfaceState) {
    for window in [
        state.window,
        HWND(state.binding.deny.control.control_window as *mut c_void),
        HWND(state.binding.allow.control.control_window as *mut c_void),
    ]
    .into_iter()
    .chain(
        state
            .checkboxes
            .iter()
            .map(|checkbox| HWND(checkbox.control.control_window as *mut c_void)),
    ) {
        if window == HWND::default() {
            continue;
        }
        let mut message = MSG::default();
        while unsafe { PeekMessageW(&mut message, Some(window), 0, 0, PM_REMOVE).as_bool() } {}
    }
}

fn reclaim_surface_after_worker_exit(reclaim: &AtomicUsize) {
    let raw = reclaim.swap(0, Ordering::AcqRel) as *mut SurfaceState;
    if raw.is_null() {
        return;
    }
    // The UI thread has been joined, so no WndProc can retain or access this
    // allocation and Windows has reclaimed every HWND owned by that thread.
    // Dropping the request now releases the prompt slot as Dismissed.
    unsafe { drop(Box::from_raw(raw)) };
}

fn callback_fail_stop(window: HWND) -> LRESULT {
    let raw = unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) } as *mut SurfaceState;
    if !raw.is_null() {
        unsafe {
            (*raw).availability.store(false, Ordering::Release);
            (*raw).decision = Some(ConsentSurfaceDecision::Dismissed);
        }
    }
    let _ = CALLBACK_FAILED.try_with(|failed| failed.set(true));
    unsafe { PostQuitMessage(1) };
    LRESULT(0)
}

fn callback_failed() -> bool {
    CALLBACK_FAILED.try_with(Cell::get).unwrap_or(true)
}

fn native_error(operation: &'static str, error: windows::core::Error) -> WindowsConsentError {
    WindowsConsentError::Native {
        operation,
        status: error.code().0,
    }
}

fn last_native_error(operation: &'static str) -> WindowsConsentError {
    native_error(operation, windows::core::Error::from_thread())
}

#[derive(Debug, Default)]
struct CloseLatch {
    active_generation: AtomicU64,
    requested_generation: AtomicU64,
}

impl CloseLatch {
    fn reserve(&self, generation: u64) -> bool {
        generation != 0
            && self
                .active_generation
                .compare_exchange(0, generation, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
    }

    fn request(&self, generation: u64) {
        if self.active_generation.load(Ordering::Acquire) == generation {
            self.requested_generation
                .fetch_max(generation, Ordering::AcqRel);
        }
    }

    fn is_requested(&self, generation: u64) -> bool {
        self.requested_generation.load(Ordering::Acquire) == generation
    }

    fn release(&self, generation: u64) {
        let _ = self.active_generation.compare_exchange(
            generation,
            0,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PromptAuthority {
    generation: u64,
    token: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ButtonAction {
    Deny,
    Allow,
}

#[derive(Debug, PartialEq, Eq)]
struct ControlBinding {
    authority: PromptAuthority,
    surface: PromptAuthority,
    parent_window: isize,
    control_id: u16,
    control_window: isize,
}

#[derive(Debug, PartialEq, Eq)]
struct ButtonBinding {
    control: ControlBinding,
    action: ButtonAction,
}

#[derive(Debug, PartialEq, Eq)]
struct CommandBinding {
    surface: PromptAuthority,
    deny: ButtonBinding,
    allow: ButtonBinding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NativeCommand {
    Deny(u64),
    Allow(u64),
    Dismiss(u64),
    Ignore,
}

fn classify_command(
    binding: &CommandBinding,
    trusted: Option<PromptAuthority>,
    command_id: u16,
    notification: u16,
    source_window: isize,
) -> NativeCommand {
    for button in [&binding.deny, &binding.allow] {
        if notification == BN_CLICKED as u16
            && command_id == button.control.control_id
            && source_window == button.control.control_window
        {
            return if trusted == Some(button.control.authority) {
                match button.action {
                    ButtonAction::Deny => NativeCommand::Deny(button.control.authority.generation),
                    ButtonAction::Allow => {
                        NativeCommand::Allow(button.control.authority.generation)
                    }
                }
            } else {
                // A posted or late WM_COMMAND has no synchronous subclass
                // provenance. Ignoring it prevents reused HWND values from
                // deciding a later prompt.
                NativeCommand::Ignore
            };
        }
    }
    NativeCommand::Dismiss(binding.surface.generation)
}

fn classify_close(binding: &CommandBinding, trusted: Option<PromptAuthority>) -> NativeCommand {
    if trusted == Some(binding.surface) {
        NativeCommand::Dismiss(binding.surface.generation)
    } else {
        NativeCommand::Ignore
    }
}

fn destroy_is_authorized(binding: &CommandBinding, trusted: Option<PromptAuthority>) -> bool {
    trusted == Some(binding.surface)
}

#[cfg(test)]
mod tests {
    use super::{
        classify_close, classify_command, control_message_is_authorized, destroy_is_authorized,
        explicit_authority_is_current, fingerprint_utf16, is_exact_scope_toggle, keyboard_decision,
        prefixed_utf16, retain_owner_until_destroyed, surface_height, system_close_is_authorized,
        ButtonAction, ButtonBinding, CloseLatch, CommandBinding, ControlBinding,
        InputMessageOrigin, NativeCommand, PromptAuthority, ScopeCheckbox,
        WindowsConsentSurfaceDriver, VK_ESCAPE_CODE, VK_RETURN_CODE,
        WM_MRD_QUERY_CONTROL_AUTHORITY, WM_MRD_QUERY_SURFACE_AUTHORITY, WM_MRD_TEST_CONTROL_ACTION,
        WM_MRD_TEST_SURFACE_DISMISS,
    };
    use crate::consent::{
        ConsentBackend, ConsentBackendDecision, ConsentBackendFuture, ConsentPrompt,
    };
    use crate::native_consent::NativeConsentBackend;
    use mrd_agent_ipc::PeerBinding;
    use mrd_proto::{DeviceId, SessionId};
    use mrd_session::PermissionScope;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, Mutex};
    use std::task::{Context, Poll, Wake, Waker};
    use std::thread;
    use std::time::Duration;
    use tokio::sync::watch;
    use windows::core::BOOL;
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumThreadWindows, GetDlgItem, GetWindowLongPtrW, IsWindow, PostMessageW, SendMessageW,
        BM_CLICK, BM_GETCHECK, BM_SETCHECK, BS_DEFPUSHBUTTON, GWL_STYLE, SC_CLOSE, WM_CLOSE,
        WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_SYSCOMMAND,
    };

    const GENERATION: u64 = 41;

    fn binding(generation: u64, token_base: usize) -> CommandBinding {
        CommandBinding {
            surface: PromptAuthority {
                generation,
                token: token_base,
            },
            deny: ButtonBinding {
                control: ControlBinding {
                    authority: PromptAuthority {
                        generation,
                        token: token_base + 1,
                    },
                    surface: PromptAuthority {
                        generation,
                        token: token_base,
                    },
                    parent_window: 99,
                    control_id: 10,
                    control_window: 100,
                },
                action: ButtonAction::Deny,
            },
            allow: ButtonBinding {
                control: ControlBinding {
                    authority: PromptAuthority {
                        generation,
                        token: token_base + 2,
                    },
                    surface: PromptAuthority {
                        generation,
                        token: token_base,
                    },
                    parent_window: 99,
                    control_id: 11,
                    control_window: 101,
                },
                action: ButtonAction::Allow,
            },
        }
    }

    unsafe extern "system" fn capture_thread_window(window: HWND, state: LPARAM) -> BOOL {
        let state = state.0 as *mut Option<HWND>;
        if !state.is_null() {
            unsafe { *state = Some(window) };
        }
        BOOL(0)
    }

    fn first_thread_window(thread_id: u32) -> Option<HWND> {
        let mut window = None;
        unsafe {
            let _ = EnumThreadWindows(
                thread_id,
                Some(capture_thread_window),
                LPARAM(std::ptr::from_mut(&mut window) as isize),
            );
        }
        window
    }

    fn query_authority(window: HWND, message: u32) -> PromptAuthority {
        let generation =
            unsafe { SendMessageW(window, message, Some(WPARAM(0)), Some(LPARAM(0))).0 } as u64;
        let token =
            unsafe { SendMessageW(window, message, Some(WPARAM(1)), Some(LPARAM(0))).0 } as usize;
        PromptAuthority { generation, token }
    }

    fn invoke_exact_control_action(window: HWND, authority: PromptAuthority) {
        let acknowledged = unsafe {
            SendMessageW(
                window,
                WM_MRD_TEST_CONTROL_ACTION,
                Some(WPARAM(authority.generation as usize)),
                Some(LPARAM(authority.token as isize)),
            )
            .0
        };
        assert_eq!(
            acknowledged, 1,
            "exact test control action was not accepted"
        );
    }

    fn invoke_exact_surface_dismiss(window: HWND, authority: PromptAuthority) {
        unsafe {
            SendMessageW(
                window,
                WM_MRD_TEST_SURFACE_DISMISS,
                Some(WPARAM(authority.generation as usize)),
                Some(LPARAM(authority.token as isize)),
            );
        }
    }

    fn wait_for_decision(
        future: &mut ConsentBackendFuture,
        waker: &Waker,
    ) -> ConsentBackendDecision {
        for _ in 0..200 {
            if let Poll::Ready(decision) =
                Future::poll(Pin::as_mut(future), &mut Context::from_waker(waker))
            {
                return decision;
            }
            thread::sleep(Duration::from_millis(5));
        }
        panic!("native consent decision did not complete")
    }

    struct DropBackendOnWake(Mutex<Option<NativeConsentBackend>>);

    impl Wake for DropBackendOnWake {
        fn wake(self: Arc<Self>) {
            drop(self.0.lock().unwrap().take());
        }
    }

    #[test]
    fn only_exact_current_button_bindings_can_decide() {
        let binding = binding(GENERATION, 500);
        assert_eq!(
            classify_command(&binding, Some(binding.deny.control.authority), 10, 0, 100,),
            NativeCommand::Deny(GENERATION)
        );
        assert_eq!(
            classify_command(&binding, Some(binding.allow.control.authority), 11, 0, 101,),
            NativeCommand::Allow(GENERATION)
        );
    }

    #[test]
    fn missing_or_stale_button_provenance_cannot_decide_a_reused_hwnd() {
        let old = binding(GENERATION, 500);
        let current = binding(GENERATION + 1, 600);
        assert_eq!(
            classify_command(&current, None, 11, 0, 101),
            NativeCommand::Ignore
        );
        assert!(!control_message_is_authorized(
            &current.allow.control,
            None,
            BM_CLICK,
            0,
            InputMessageOrigin::Unavailable,
        ));
        assert!(!control_message_is_authorized(
            &current.allow.control,
            Some(old.surface),
            BM_CLICK,
            0,
            InputMessageOrigin::Hardware,
        ));
        assert!(!control_message_is_authorized(
            &current.allow.control,
            Some(current.surface),
            BM_CLICK,
            0,
            InputMessageOrigin::Hardware,
        ));
        assert!(!control_message_is_authorized(
            &current.allow.control,
            Some(current.surface),
            BM_SETCHECK,
            1,
            InputMessageOrigin::Hardware,
        ));
        assert!(!explicit_authority_is_current(
            current.allow.control.authority,
            old.allow.control.authority.generation,
            old.allow.control.authority.token,
        ));
        assert!(explicit_authority_is_current(
            current.allow.control.authority,
            current.allow.control.authority.generation,
            current.allow.control.authority.token,
        ));
        assert_eq!(
            classify_command(&current, Some(old.allow.control.authority), 11, 0, 101,),
            NativeCommand::Ignore
        );
    }

    #[test]
    fn only_hardware_mouse_or_space_input_can_mint_control_authority() {
        let current = binding(GENERATION, 500);
        for origin in [
            InputMessageOrigin::Unavailable,
            InputMessageOrigin::Injected,
            InputMessageOrigin::System,
        ] {
            assert!(!control_message_is_authorized(
                &current.allow.control,
                Some(current.surface),
                WM_LBUTTONUP,
                0,
                origin,
            ));
            assert!(!control_message_is_authorized(
                &current.allow.control,
                Some(current.surface),
                WM_KEYUP,
                super::VK_SPACE_CODE,
                origin,
            ));
        }
        assert!(control_message_is_authorized(
            &current.allow.control,
            Some(current.surface),
            WM_LBUTTONUP,
            0,
            InputMessageOrigin::Hardware,
        ));
        assert!(control_message_is_authorized(
            &current.allow.control,
            Some(current.surface),
            WM_KEYUP,
            super::VK_SPACE_CODE,
            InputMessageOrigin::Hardware,
        ));
    }

    #[test]
    fn close_requires_hardware_origin_and_exact_current_surface() {
        let old = binding(GENERATION, 500);
        let current = binding(GENERATION + 1, 600);
        assert!(!system_close_is_authorized(
            &current,
            Some(current.surface),
            InputMessageOrigin::Unavailable,
        ));
        assert!(!system_close_is_authorized(
            &current,
            Some(old.surface),
            InputMessageOrigin::Hardware,
        ));
        assert!(system_close_is_authorized(
            &current,
            Some(current.surface),
            InputMessageOrigin::Hardware,
        ));
    }

    #[test]
    fn unknown_current_command_fails_closed() {
        let binding = binding(GENERATION, 500);
        assert_eq!(
            classify_command(&binding, Some(binding.allow.control.authority), 11, 0, 999,),
            NativeCommand::Dismiss(GENERATION)
        );
        assert_eq!(
            classify_command(&binding, Some(binding.allow.control.authority), 999, 0, 101,),
            NativeCommand::Dismiss(GENERATION)
        );
        assert_eq!(
            classify_command(&binding, Some(binding.allow.control.authority), 11, 1, 101,),
            NativeCommand::Dismiss(GENERATION)
        );
    }

    #[test]
    fn close_and_destroy_require_exact_current_surface_provenance() {
        let old = binding(GENERATION, 500);
        let current = binding(GENERATION + 1, 600);
        assert_eq!(classify_close(&current, None), NativeCommand::Ignore);
        assert_eq!(
            classify_close(&current, Some(old.surface)),
            NativeCommand::Ignore
        );
        assert_eq!(
            classify_close(&current, Some(current.surface)),
            NativeCommand::Dismiss(GENERATION + 1)
        );
        assert!(!destroy_is_authorized(&current, None));
        assert!(!destroy_is_authorized(&current, Some(old.surface)));
        assert!(destroy_is_authorized(&current, Some(current.surface)));
    }

    #[test]
    fn exact_scope_checkbox_click_is_non_terminal_but_mismatches_are_not() {
        let checkbox = ScopeCheckbox {
            scope: PermissionScope::ScreenView,
            control: ControlBinding {
                authority: PromptAuthority {
                    generation: GENERATION,
                    token: 501,
                },
                surface: PromptAuthority {
                    generation: GENERATION,
                    token: 500,
                },
                parent_window: 99,
                control_id: 20,
                control_window: 200,
            },
        };
        let checkboxes = [checkbox];
        assert!(is_exact_scope_toggle(&checkboxes, 20, 0, 200));
        assert!(!is_exact_scope_toggle(&checkboxes, 20, 1, 200));
        assert!(!is_exact_scope_toggle(&checkboxes, 20, 0, 201));
        assert!(!is_exact_scope_toggle(&checkboxes, 21, 0, 200));
    }

    #[test]
    fn escape_dismisses_and_enter_uses_the_default_deny_path() {
        assert_eq!(
            keyboard_decision(WM_KEYDOWN, VK_ESCAPE_CODE, InputMessageOrigin::Hardware,),
            Some(super::ConsentSurfaceDecision::Dismissed)
        );
        assert_eq!(
            keyboard_decision(WM_KEYDOWN, VK_RETURN_CODE, InputMessageOrigin::Hardware,),
            Some(super::ConsentSurfaceDecision::Denied)
        );
        assert_eq!(
            keyboard_decision(WM_KEYDOWN, VK_RETURN_CODE, InputMessageOrigin::Injected,),
            None
        );
        assert_eq!(
            keyboard_decision(WM_KEYUP, VK_RETURN_CODE, InputMessageOrigin::Hardware),
            None
        );
    }

    #[test]
    fn close_latch_cannot_be_lost_or_retargeted_by_a_stale_generation() {
        let latch = CloseLatch::default();
        assert!(latch.reserve(41));
        latch.request(41);
        assert!(latch.is_requested(41));
        latch.release(41);

        assert!(latch.reserve(42));
        latch.request(41);
        assert!(!latch.is_requested(42));
        latch.request(42);
        assert!(latch.is_requested(42));
        latch.release(42);
    }

    #[test]
    fn failed_destroy_keeps_the_owner_until_a_proven_success() {
        let mut owner = Some(41_u64);
        assert!(!retain_owner_until_destroyed(&mut owner, |_| false));
        assert_eq!(owner, Some(41));

        assert!(retain_owner_until_destroyed(&mut owner, |generation| {
            generation == 41
        }));
        assert_eq!(owner, None);
    }

    #[test]
    fn rendered_fingerprint_contains_every_byte_in_fixed_order() {
        let fingerprint = std::array::from_fn(|index| index as u8);
        let rendered = fingerprint_utf16(&fingerprint);
        let decoded = rendered
            .each_ref()
            .map(|line| String::from_utf16(&line[..line.len() - 1]).unwrap());
        assert_eq!(
            decoded,
            [
                "Key SHA-256 (1/2): 00:01:02:03:04:05:06:07:08:09:0A:0B:0C:0D:0E:0F",
                "Key SHA-256 (2/2): 10:11:12:13:14:15:16:17:18:19:1A:1B:1C:1D:1E:1F",
            ]
        );
        assert!(rendered.iter().all(|line| {
            line.last() == Some(&0) && line.iter().filter(|unit| **unit == 0).count() == 1
        }));
    }

    #[test]
    fn all_permission_rows_fit_in_a_bounded_two_column_surface() {
        assert_eq!(surface_height(0), 276);
        assert_eq!(surface_height(1), 304);
        assert_eq!(surface_height(18), 528);
    }

    #[test]
    fn prefixed_display_text_has_exactly_one_terminal_nul() {
        let rendered = prefixed_utf16("Session: ", &[b'a' as u16, b'b' as u16]);
        assert_eq!(rendered.last(), Some(&0));
        assert_eq!(rendered.iter().filter(|unit| **unit == 0).count(), 1);
    }

    #[test]
    #[ignore = "opens and drives a native Windows consent surface"]
    fn native_checkbox_subset_and_allow_button_complete_after_destroy() {
        let (driver, availability) = WindowsConsentSurfaceDriver::start().unwrap();
        let backend = NativeConsentBackend::new(driver.clone(), Arc::clone(&availability));
        let (abort, _) = watch::channel(None);
        let requested = [PermissionScope::ScreenView].into_iter().collect();
        let mut future = backend.prompt(
            ConsentPrompt::for_native_test(
                SessionId("native-smoke".into()),
                PeerBinding {
                    device_id: DeviceId("peer".into()),
                    key_id: [0xabu8; 32],
                },
                requested,
            ),
            abort.subscribe(),
        );
        let waker = Waker::noop();
        assert_eq!(
            Future::poll(Pin::as_mut(&mut future), &mut Context::from_waker(waker)),
            Poll::Pending
        );

        let mut controls = None;
        let mut saw_surface = false;
        let mut saw_checkbox = false;
        let mut saw_allow = false;
        for _ in 0..200 {
            controls = (|| {
                let surface = first_thread_window(driver.shared.worker.thread_id)?;
                saw_surface = true;
                let checkbox =
                    unsafe { GetDlgItem(Some(surface), i32::from(super::FIRST_SCOPE_CONTROL_ID)) }
                        .ok()?;
                saw_checkbox = true;
                let allow =
                    unsafe { GetDlgItem(Some(surface), i32::from(super::ALLOW_CONTROL_ID)) }
                        .ok()?;
                saw_allow = true;
                let deny =
                    unsafe { GetDlgItem(Some(surface), i32::from(super::DENY_CONTROL_ID)) }.ok()?;
                Some((surface, checkbox, deny, allow))
            })();
            if controls.is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        let (surface, checkbox, deny, allow) = controls.unwrap_or_else(|| {
            let outcome = Future::poll(
                Pin::as_mut(&mut future),
                &mut Context::from_waker(waker),
            );
            let supervisor_finished = driver
                .join
                .lock()
                .unwrap()
                .as_ref()
                .is_none_or(|join| join.is_finished());
            panic!(
                "native consent controls did not become ready: surface={saw_surface}, checkbox={saw_checkbox}, allow={saw_allow}, available={}, shutdown={}, reclaim={}, supervisor_finished={supervisor_finished}, future={outcome:?}",
                availability.load(Ordering::Acquire),
                driver.shared.shutdown.load(Ordering::Acquire),
                driver.shared.reclaim.load(Ordering::Acquire),
            )
        });
        let first_surface_authority = query_authority(surface, WM_MRD_QUERY_SURFACE_AUTHORITY);
        let first_checkbox_authority = query_authority(checkbox, WM_MRD_QUERY_CONTROL_AUTHORITY);
        let first_allow_authority = query_authority(allow, WM_MRD_QUERY_CONTROL_AUTHORITY);

        let deny_style = unsafe { GetWindowLongPtrW(deny, GWL_STYLE) } as u32;
        assert_ne!(
            deny_style & u32::try_from(BS_DEFPUSHBUTTON).unwrap_or(0),
            0,
            "Deny must be the default push button"
        );
        assert_eq!(
            unsafe {
                SendMessageW(
                    surface,
                    super::WM_MRD_QUERY_INITIAL_FOCUS,
                    Some(WPARAM(0)),
                    Some(LPARAM(0)),
                )
                .0
            },
            1,
            "Deny must receive verified initial focus"
        );

        unsafe {
            SendMessageW(checkbox, BM_CLICK, Some(WPARAM(0)), Some(LPARAM(0)));
        }
        assert_eq!(
            unsafe { SendMessageW(checkbox, BM_GETCHECK, Some(WPARAM(0)), Some(LPARAM(0))).0 },
            0,
            "a sent control message has no generation dispatch authority"
        );
        assert!(unsafe { IsWindow(Some(surface)).as_bool() });
        unsafe {
            SendMessageW(allow, BM_CLICK, Some(WPARAM(0)), Some(LPARAM(0)));
        }
        assert!(unsafe { IsWindow(Some(surface)).as_bool() });
        assert_eq!(
            Future::poll(Pin::as_mut(&mut future), &mut Context::from_waker(waker)),
            Poll::Pending
        );
        unsafe {
            // Posted standard control messages never carry local-input or
            // prompt-generation authority.
            PostMessageW(Some(checkbox), BM_CLICK, WPARAM(0), LPARAM(0)).unwrap();
            PostMessageW(Some(allow), BM_CLICK, WPARAM(0), LPARAM(0)).unwrap();
            PostMessageW(Some(checkbox), WM_LBUTTONDOWN, WPARAM(0), LPARAM(0)).unwrap();
            PostMessageW(Some(checkbox), WM_LBUTTONUP, WPARAM(0), LPARAM(0)).unwrap();
            PostMessageW(
                Some(allow),
                WM_KEYDOWN,
                WPARAM(super::VK_SPACE_CODE),
                LPARAM(0),
            )
            .unwrap();
            PostMessageW(
                Some(allow),
                WM_KEYUP,
                WPARAM(super::VK_SPACE_CODE),
                LPARAM(0),
            )
            .unwrap();
        }
        thread::sleep(Duration::from_millis(25));
        assert_eq!(
            unsafe { SendMessageW(checkbox, BM_GETCHECK, Some(WPARAM(0)), Some(LPARAM(0))).0 },
            0
        );
        assert!(unsafe { IsWindow(Some(surface)).as_bool() });
        assert_eq!(
            Future::poll(Pin::as_mut(&mut future), &mut Context::from_waker(waker)),
            Poll::Pending
        );
        invoke_exact_control_action(checkbox, first_checkbox_authority);
        invoke_exact_control_action(allow, first_allow_authority);
        assert_eq!(
            wait_for_decision(&mut future, waker),
            ConsentBackendDecision::Approved([PermissionScope::ScreenView].into_iter().collect())
        );
        assert!(!unsafe { IsWindow(Some(surface)).as_bool() });

        let mut second = backend.prompt(
            ConsentPrompt::for_native_test(
                SessionId("native-smoke-2".into()),
                PeerBinding {
                    device_id: DeviceId("peer".into()),
                    key_id: [0xabu8; 32],
                },
                [PermissionScope::ScreenView].into_iter().collect(),
            ),
            abort.subscribe(),
        );
        assert_eq!(
            Future::poll(Pin::as_mut(&mut second), &mut Context::from_waker(waker)),
            Poll::Pending
        );
        let mut second_controls = None;
        for _ in 0..200 {
            second_controls =
                first_thread_window(driver.shared.worker.thread_id).and_then(|window| {
                    let deny =
                        unsafe { GetDlgItem(Some(window), i32::from(super::DENY_CONTROL_ID)) }
                            .ok()?;
                    Some((window, deny))
                });
            if second_controls.is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        let (second_surface, second_deny) = second_controls.unwrap_or_else(|| {
            let outcome = Future::poll(
                Pin::as_mut(&mut second),
                &mut Context::from_waker(waker),
            );
            let supervisor_finished = driver
                .join
                .lock()
                .unwrap()
                .as_ref()
                .is_none_or(|join| join.is_finished());
            let failure = *driver.shared.failure.lock().unwrap();
            panic!(
                "second native consent surface did not open: available={}, shutdown={}, reclaim={}, supervisor_finished={supervisor_finished}, failure={failure:?}, future={outcome:?}",
                availability.load(Ordering::Acquire),
                driver.shared.shutdown.load(Ordering::Acquire),
                driver.shared.reclaim.load(Ordering::Acquire),
            )
        });
        let second_deny_authority = query_authority(second_deny, WM_MRD_QUERY_CONTROL_AUTHORITY);
        // Simulate an old HWND value resolving to the new control: a standard
        // late callback and an explicit callback carrying generation one's
        // nonce must both remain powerless after generation two is visible.
        unsafe {
            PostMessageW(Some(second_deny), BM_CLICK, WPARAM(0), LPARAM(0)).unwrap();
            PostMessageW(
                Some(second_deny),
                WM_KEYDOWN,
                WPARAM(VK_RETURN_CODE),
                LPARAM(0),
            )
            .unwrap();
            PostMessageW(
                Some(second_deny),
                WM_KEYDOWN,
                WPARAM(VK_ESCAPE_CODE),
                LPARAM(0),
            )
            .unwrap();
            SendMessageW(
                second_deny,
                WM_MRD_TEST_CONTROL_ACTION,
                Some(WPARAM(first_allow_authority.generation as usize)),
                Some(LPARAM(first_allow_authority.token as isize)),
            );
        }
        thread::sleep(Duration::from_millis(25));
        assert!(unsafe { IsWindow(Some(second_surface)).as_bool() });
        assert_eq!(
            Future::poll(Pin::as_mut(&mut second), &mut Context::from_waker(waker)),
            Poll::Pending
        );
        invoke_exact_control_action(second_deny, second_deny_authority);
        assert_eq!(
            wait_for_decision(&mut second, waker),
            ConsentBackendDecision::Denied
        );

        let mut third = backend.prompt(
            ConsentPrompt::for_native_test(
                SessionId("native-smoke-3".into()),
                PeerBinding {
                    device_id: DeviceId("peer".into()),
                    key_id: [0xabu8; 32],
                },
                [PermissionScope::ScreenView].into_iter().collect(),
            ),
            abort.subscribe(),
        );
        assert_eq!(
            Future::poll(Pin::as_mut(&mut third), &mut Context::from_waker(waker)),
            Poll::Pending
        );
        let mut third_surface = None;
        for _ in 0..200 {
            third_surface = first_thread_window(driver.shared.worker.thread_id);
            if third_surface.is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        let third_surface = third_surface.expect("third native consent surface did not open");
        let third_surface_authority =
            query_authority(third_surface, WM_MRD_QUERY_SURFACE_AUTHORITY);
        unsafe {
            SendMessageW(third_surface, WM_CLOSE, Some(WPARAM(0)), Some(LPARAM(0)));
        }
        assert!(unsafe { IsWindow(Some(third_surface)).as_bool() });
        unsafe {
            SendMessageW(
                third_surface,
                WM_SYSCOMMAND,
                Some(WPARAM(SC_CLOSE as usize)),
                Some(LPARAM(0)),
            );
        }
        assert!(unsafe { IsWindow(Some(third_surface)).as_bool() });
        unsafe {
            PostMessageW(
                Some(third_surface),
                WM_SYSCOMMAND,
                WPARAM(SC_CLOSE as usize),
                LPARAM(0),
            )
            .unwrap();
            SendMessageW(
                third_surface,
                WM_MRD_TEST_SURFACE_DISMISS,
                Some(WPARAM(first_surface_authority.generation as usize)),
                Some(LPARAM(first_surface_authority.token as isize)),
            );
        }
        thread::sleep(Duration::from_millis(25));
        assert!(unsafe { IsWindow(Some(third_surface)).as_bool() });
        assert_eq!(
            Future::poll(Pin::as_mut(&mut third), &mut Context::from_waker(waker)),
            Poll::Pending
        );
        invoke_exact_surface_dismiss(third_surface, third_surface_authority);
        assert_eq!(
            wait_for_decision(&mut third, waker),
            ConsentBackendDecision::Dismissed
        );
        assert!(!unsafe { IsWindow(Some(third_surface)).as_bool() });

        let mut fourth = backend.prompt(
            ConsentPrompt::for_native_test(
                SessionId("native-smoke-4".into()),
                PeerBinding {
                    device_id: DeviceId("peer".into()),
                    key_id: [0xabu8; 32],
                },
                [PermissionScope::ScreenView].into_iter().collect(),
            ),
            abort.subscribe(),
        );
        assert_eq!(
            Future::poll(Pin::as_mut(&mut fourth), &mut Context::from_waker(waker)),
            Poll::Pending
        );
        let mut fourth_surface = None;
        for _ in 0..200 {
            fourth_surface = first_thread_window(driver.shared.worker.thread_id);
            if fourth_surface.is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        let fourth_surface = fourth_surface.expect("fourth native consent surface did not open");
        drop(backend);
        assert!(!availability.load(Ordering::Acquire));
        assert!(!unsafe { IsWindow(Some(fourth_surface)).as_bool() });
        assert_eq!(
            Future::poll(Pin::as_mut(&mut fourth), &mut Context::from_waker(waker)),
            Poll::Ready(ConsentBackendDecision::Dismissed)
        );
    }

    #[test]
    #[ignore = "opens a native Windows consent surface with a reentrant completion waker"]
    fn completion_waker_can_drop_backend_without_self_joining_the_ui_thread() {
        let (driver, availability) = WindowsConsentSurfaceDriver::start().unwrap();
        let backend = NativeConsentBackend::new(driver.clone(), Arc::clone(&availability));
        let (abort, _) = watch::channel(None);
        let mut future = backend.prompt(
            ConsentPrompt::for_native_test(
                SessionId("reentrant-waker".into()),
                PeerBinding {
                    device_id: DeviceId("peer".into()),
                    key_id: [0xabu8; 32],
                },
                [PermissionScope::ScreenView].into_iter().collect(),
            ),
            abort.subscribe(),
        );
        let drop_on_wake = Arc::new(DropBackendOnWake(Mutex::new(Some(backend))));
        let waker = Waker::from(Arc::clone(&drop_on_wake));
        assert_eq!(
            Future::poll(Pin::as_mut(&mut future), &mut Context::from_waker(&waker),),
            Poll::Pending
        );

        let mut deny = None;
        for _ in 0..200 {
            deny = first_thread_window(driver.shared.worker.thread_id).and_then(|window| {
                unsafe { GetDlgItem(Some(window), i32::from(super::DENY_CONTROL_ID)) }.ok()
            });
            if deny.is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        let deny = deny.expect("reentrant-waker consent surface did not open");
        let deny_authority = query_authority(deny, WM_MRD_QUERY_CONTROL_AUTHORITY);
        invoke_exact_control_action(deny, deny_authority);

        assert_eq!(
            wait_for_decision(&mut future, &waker),
            ConsentBackendDecision::Denied
        );
        assert!(drop_on_wake.0.lock().unwrap().is_none());
        assert!(!availability.load(Ordering::Acquire));
        for _ in 0..200 {
            if driver
                .join
                .lock()
                .unwrap()
                .as_ref()
                .is_none_or(|join| join.is_finished())
            {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        assert!(driver
            .join
            .lock()
            .unwrap()
            .as_ref()
            .is_none_or(|join| join.is_finished()));
        drop(driver);
    }

    #[test]
    #[ignore = "requires a Windows desktop process"]
    fn idle_native_worker_starts_and_is_joined_on_final_drop() {
        let (driver, availability) = WindowsConsentSurfaceDriver::start().unwrap();
        assert!(availability.load(Ordering::Acquire));
        drop(driver);
        assert!(!availability.load(Ordering::Acquire));
    }
}

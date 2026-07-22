#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DynamicWindowFpsTier {
    Active,
    Warm,
    Idle,
    Suspended,
}

impl DynamicWindowFpsTier {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Warm => "warm",
            Self::Idle => "idle",
            Self::Suspended => "suspended",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct DynamicWindowFpsDecision {
    pub(super) tier: DynamicWindowFpsTier,
    pub(super) target_fps: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct DynamicWindowFpsInput {
    pub(super) frame_changed: bool,
    pub(super) input_active: bool,
    pub(super) source_available: bool,
    pub(super) active_window_capture_count: u32,
}

pub(super) struct DynamicWindowFpsPolicy {
    profile_fps: u32,
    quiet_updates: u32,
    decision: DynamicWindowFpsDecision,
}

impl DynamicWindowFpsPolicy {
    pub(super) fn new(profile_fps: u32) -> Self {
        Self {
            profile_fps,
            quiet_updates: 0,
            decision: DynamicWindowFpsDecision {
                tier: DynamicWindowFpsTier::Active,
                target_fps: profile_fps,
            },
        }
    }

    pub(super) fn update(&mut self, input: DynamicWindowFpsInput) -> DynamicWindowFpsDecision {
        if !input.source_available {
            self.quiet_updates = 0;
            self.decision = DynamicWindowFpsDecision {
                tier: DynamicWindowFpsTier::Suspended,
                target_fps: 1,
            };
            return self.decision;
        }

        if input.frame_changed || input.input_active {
            self.quiet_updates = 0;
            let target_fps = if input.active_window_capture_count >= 3 {
                self.profile_fps.min(60)
            } else {
                self.profile_fps
            };
            self.decision = DynamicWindowFpsDecision {
                tier: DynamicWindowFpsTier::Active,
                target_fps,
            };
            return self.decision;
        }

        self.quiet_updates = self.quiet_updates.saturating_add(1);
        self.decision = if self.quiet_updates >= 10 {
            DynamicWindowFpsDecision {
                tier: DynamicWindowFpsTier::Idle,
                target_fps: self.profile_fps.min(15),
            }
        } else {
            DynamicWindowFpsDecision {
                tier: DynamicWindowFpsTier::Warm,
                target_fps: self.profile_fps.min(60),
            }
        };
        self.decision
    }

    pub(super) fn current(&self) -> DynamicWindowFpsDecision {
        self.decision
    }
}

pub(super) fn window_dynamic_fps_input(
    frame_changed: bool,
    source_available: bool,
    active_window_capture_count: u32,
) -> DynamicWindowFpsInput {
    DynamicWindowFpsInput {
        frame_changed,
        input_active: false,
        source_available,
        active_window_capture_count: active_window_capture_count.max(1),
    }
}

pub(super) fn window_dynamic_fps_input_for_captured_frame(
    active_window_capture_count: u32,
) -> DynamicWindowFpsInput {
    window_dynamic_fps_input(true, true, active_window_capture_count)
}

pub(super) fn is_winrt_window_capture_no_frame_timeout(error: &anyhow::Error) -> bool {
    format!("{error:#}").contains("WinRT capture produced no frame within")
}

pub(super) fn window_dynamic_fps_input_for_capture_error(
    error: &anyhow::Error,
    active_window_capture_count: u32,
) -> DynamicWindowFpsInput {
    window_dynamic_fps_input(
        false,
        is_winrt_window_capture_no_frame_timeout(error),
        active_window_capture_count,
    )
}

pub(super) fn update_dynamic_window_fps_decision(
    policy: &mut Option<DynamicWindowFpsPolicy>,
    decision: &mut Option<DynamicWindowFpsDecision>,
    frame_changed: bool,
    source_available: bool,
    active_window_capture_count: u32,
) {
    if let Some(policy) = policy.as_mut() {
        *decision = Some(policy.update(window_dynamic_fps_input(
            frame_changed,
            source_available,
            active_window_capture_count,
        )));
    }
}

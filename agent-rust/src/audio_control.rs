#[derive(Debug, Clone, Copy, Default)]
pub struct AudioSession {
    pub op: u8,
    pub codec: u8,
    pub sample_rate: u32,
    pub channels: u8,
    pub frame_ms: u16,
    pub route_mode: u8,
    pub route_scope: u8,
    pub target_pid: u32,
    pub follow_children: bool,
}

#[derive(Debug, Default)]
pub struct AudioControlManager {
    last: Option<AudioSession>,
}

impl AudioControlManager {
    pub fn apply(&mut self, session: AudioSession) {
        self.last = Some(session);
    }

    pub fn latest(&self) -> Option<AudioSession> {
        self.last
    }

    pub fn apply_route(&mut self, mode: u8, scope: u8, target_pid: u32, follow_children: bool) {
        let mut s = self.last.unwrap_or_default();
        s.route_mode = mode;
        s.route_scope = scope;
        s.target_pid = target_pid;
        s.follow_children = follow_children;
        self.last = Some(s);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_last_session() {
        let mut m = AudioControlManager::default();
        m.apply(AudioSession {
            op: 1,
            codec: 2,
            sample_rate: 48_000,
            channels: 2,
            frame_ms: 20,
            route_mode: 0,
            route_scope: 0,
            target_pid: 0,
            follow_children: true,
        });
        let last = m.latest().expect("latest audio");
        assert_eq!(last.codec, 2);
        assert_eq!(last.sample_rate, 48_000);
    }
}

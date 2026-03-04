#[derive(Debug, Clone, Copy, Default)]
pub struct AudioSession {
    pub op: u8,
    pub codec: u8,
    pub sample_rate: u32,
    pub channels: u8,
    pub frame_ms: u16,
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
        });
        let last = m.latest().expect("latest audio");
        assert_eq!(last.codec, 2);
        assert_eq!(last.sample_rate, 48_000);
    }
}

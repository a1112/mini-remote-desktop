use std::collections::VecDeque;

#[derive(Debug, Default)]
pub struct ClipboardManager {
    history: VecDeque<ClipboardItem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardItem {
    pub mime: u8,
    pub bytes: Vec<u8>,
}

impl ClipboardManager {
    pub fn set(&mut self, mime: u8, bytes: Vec<u8>) {
        const HISTORY_LIMIT: usize = 16;
        self.history.push_back(ClipboardItem { mime, bytes });
        while self.history.len() > HISTORY_LIMIT {
            let _ = self.history.pop_front();
        }
    }

    pub fn latest(&self) -> Option<ClipboardItem> {
        self.history.back().cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_latest_clipboard_item() {
        let mut m = ClipboardManager::default();
        m.set(1, b"hello".to_vec());
        m.set(2, b"world".to_vec());
        let latest = m.latest().expect("latest item");
        assert_eq!(latest.mime, 2);
        assert_eq!(latest.bytes, b"world".to_vec());
    }
}

use std::collections::HashMap;

#[derive(Debug, Default)]
pub struct SecretStore {
    entries: HashMap<String, String>,
}

impl SecretStore {
    pub fn put(&mut self, key: String, value: String) {
        self.entries.insert(key, value);
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries.get(key).map(|v| v.as_str())
    }

    pub fn remove(&mut self, key: &str) {
        self.entries.remove(key);
    }
}

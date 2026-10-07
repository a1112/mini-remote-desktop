use mrd_store_sqlite::{AeadSecretProtector, PersistentStore};
use rusqlite::Connection;
use std::sync::Arc;

#[test]
fn fresh_store_uses_the_policy_sealed_v3_schema() {
    let path =
        std::env::temp_dir().join(format!("mrd-policy-schema-{}.sqlite", std::process::id()));
    let protector = Arc::new(AeadSecretProtector::from_key([85; 32]).unwrap());
    let store = PersistentStore::open(&path, protector).unwrap();
    let connection = Connection::open(&path).unwrap();
    let version: u32 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 3, "permissions require the sealed v3 store format");
    let count: u64 = connection
        .query_row("SELECT COUNT(*) FROM trust_permissions", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, 0);
    drop(connection);
    drop(store);
    std::fs::remove_file(path).unwrap();
}

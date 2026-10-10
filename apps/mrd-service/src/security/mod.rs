//! Platform secret-protection adapters for service-owned machine state.

#[cfg(target_os = "macos")]
mod macos_keychain;
#[cfg(target_os = "macos")]
mod macos_user_storage;
#[cfg(all(not(windows), not(target_os = "macos")))]
mod unsupported;
#[cfg(windows)]
mod windows_dpapi;

#[cfg(target_os = "macos")]
pub use macos_keychain::{authorize_keychain_access, platform_secret_protector};
#[cfg(target_os = "macos")]
pub use macos_user_storage::{
    ensure_protected_product_data_dir, protected_product_data_dir, verify_owner_only_file,
    verify_protected_product_data_dir,
};
#[cfg(all(not(windows), not(target_os = "macos")))]
pub use unsupported::{platform_secret_protector, UnsupportedSecretProtector};
#[cfg(windows)]
pub use windows_dpapi::{
    ensure_protected_product_data_dir, platform_secret_protector, protected_product_data_dir,
    verify_protected_product_data_dir, DpapiMachineProtector, ProductDirectoryAclPolicy,
};

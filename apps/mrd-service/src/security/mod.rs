//! Platform secret-protection adapters for service-owned machine state.

#[cfg(target_os = "macos")]
mod macos_keychain;
#[cfg(not(any(windows, target_os = "macos")))]
mod unsupported;
#[cfg(target_os = "macos")]
pub use macos_keychain::{platform_secret_protector, protected_product_data_dir};
#[cfg(windows)]
mod windows_dpapi;

#[cfg(not(any(windows, target_os = "macos")))]
pub use unsupported::{platform_secret_protector, UnsupportedSecretProtector};
#[cfg(windows)]
pub use windows_dpapi::{
    ensure_protected_product_data_dir, platform_secret_protector, protected_product_data_dir,
    verify_protected_product_data_dir, DpapiMachineProtector, ProductDirectoryAclPolicy,
};

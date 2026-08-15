//! Platform secret-protection adapters for service-owned machine state.

#[cfg(not(windows))]
mod unsupported;
#[cfg(windows)]
mod windows_dpapi;

#[cfg(not(windows))]
pub use unsupported::{platform_secret_protector, UnsupportedSecretProtector};
#[cfg(windows)]
pub use windows_dpapi::{
    ensure_protected_product_data_dir, platform_secret_protector, protected_product_data_dir,
    verify_protected_product_data_dir, DpapiMachineProtector, ProductDirectoryAclPolicy,
};

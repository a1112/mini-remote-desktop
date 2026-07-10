#![allow(missing_docs)]

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PermissionScope {
    ScreenView,
    InputPointer,
    InputKeyboard,
    ClipboardRead,
    ClipboardWrite,
    FileRead,
    FileWrite,
    AudioListen,
    AudioTalk,
    PowerAction,
    SecureDesktopView,
    SecureDesktopControl,
}

pub type PermissionScopes = BTreeSet<PermissionScope>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveScopes(PermissionScopes);

impl EffectiveScopes {
    pub fn resolve(
        requested: PermissionScopes,
        peer_maximum: PermissionScopes,
        local_policy: PermissionScopes,
        local_approval: PermissionScopes,
        runtime_capabilities: PermissionScopes,
    ) -> Self {
        let mut result = requested;
        for other in [peer_maximum, local_policy, local_approval, runtime_capabilities] {
            result = result.intersection(&other).copied().collect();
        }
        Self(result)
    }

    pub fn into_inner(self) -> PermissionScopes { self.0 }
}

use mrd_proto::{DeviceId, SessionId};
use mrd_session::{EffectiveScopes, GrantError, PermissionScope, PermissionScopes, SessionGrant};

fn scopes(values: &[PermissionScope]) -> PermissionScopes {
    values.iter().copied().collect()
}

#[test]
fn effective_scopes_are_the_strict_intersection() {
    let effective = EffectiveScopes::resolve(
        scopes(&[PermissionScope::ScreenView, PermissionScope::InputKeyboard, PermissionScope::FileWrite]),
        scopes(&[PermissionScope::ScreenView, PermissionScope::InputKeyboard]),
        scopes(&[PermissionScope::ScreenView]),
        scopes(&[PermissionScope::ScreenView, PermissionScope::InputKeyboard]),
        scopes(&[PermissionScope::ScreenView, PermissionScope::InputKeyboard]),
    );
    assert_eq!(effective.into_inner(), scopes(&[PermissionScope::ScreenView]));
}

#[test]
fn expired_grant_cannot_authorize_input() {
    let grant = SessionGrant::new(
        SessionId("session-1".into()),
        DeviceId("peer-1".into()),
        scopes(&[PermissionScope::InputKeyboard]),
        10,
        100,
        [7; 16],
    );
    assert_eq!(grant.authorize(PermissionScope::InputKeyboard, 101), Err(GrantError::Expired));
}

#[test]
fn grant_rejects_scope_not_in_payload() {
    let grant = SessionGrant::new(
        SessionId("session-1".into()),
        DeviceId("peer-1".into()),
        scopes(&[PermissionScope::ScreenView]),
        10,
        100,
        [7; 16],
    );
    assert_eq!(grant.authorize(PermissionScope::InputKeyboard, 20), Err(GrantError::ScopeNotGranted));
}

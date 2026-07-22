# Sidebar Device Actions Design

## Goal

Make the Sidebar device context menu honest: actions must either call a real service path or be visibly unavailable.

## Design

The Sidebar keeps navigation and local UI actions that already work: open remote desktop, rename, copy device ID, and refresh after successful changes. The "exit binding" action will call `deviceService.unbindDevice(userId, deviceId)`, using the authenticated user from `useAuth` and the selected device ID from the menu.

Actions without a current backend or local service contract will no longer close silently or show "simulated operation" alerts. Those entries will render disabled with a short unavailable reason in the title. This covers file transfer, remote terminal, favorite, disable, disconnect, remove, and management actions.

## Error Handling

If the user is not logged in, "exit binding" is disabled. If the API call fails, Sidebar shows a compact inline status message instead of using `alert()`.

## Tests

Add Sidebar component tests that mock device discovery, auth state, and `deviceService`. The tests verify unsupported actions are disabled and that a logged-in exit binding call passes the selected device ID to the service.

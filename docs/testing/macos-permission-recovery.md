# Service-owned macOS permission recovery

The permission panel continues to read the running service's capability checks. Rdesk GUI trust is not substituted for mrd-service trust. The existing shell-status IPC now includes the service's actual current executable path and PID; old services that omit the path remain supported and the UI explicitly reports an unknown path. A path is diagnostic identity, not proof of code-signing identity or TCC attribution.

For macOS, the panel explains Screen Recording and Accessibility separately and opens only those allowlisted Settings destinations. Opening Settings neither requests nor grants permission. macOS deep links are best-effort navigation; failure falls back to opening System Settings, and manual navigation remains visible. The panel rechecks on focus/visibility return and provides explicit retry. It does not prompt at startup, restart automatically, or require control permission for view-only use.

If macOS explicitly asks to quit and reopen, end sessions and save work first, then quit both the UI and service. Merely closing the UI window may leave the service alive. A missing permission or cached snapshot alone is not evidence that restart is required. A capability preflight is not proof that a subsequent real stream succeeds.

The packaged service has its own ScreenCapture usage description. Verify the installed, signed Rdesk Service.app and parent app on macOS: correct service path/PID; fresh, denied and revoked screen/control access; Settings return; old-service identity omission; service surviving UI close; actual capture/control errors after a grant; system-required restart without losing work. No Linux test can verify TCC attribution, signing, actual Settings anchors, or a real capture/control session.

Sources: [Apple AX status](https://developer.apple.com/documentation/applicationservices/1459186-axisprocesstrustedwithoptions), [Apple capture sample and restart](https://developer.apple.com/documentation/screencapturekit/capturing-screen-content-in-macos), [Apple unsupported URL scheme caveat](https://developer.apple.com/forums/thread/761314).

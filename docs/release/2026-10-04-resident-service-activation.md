# Windows resident service activation — 2026-10-04

The user authorized replacement of the installed service and committing/pushing the completed changes.

## Activated and verified

- Installed `mrd-service.exe` SHA256: `bcef66551ca0d915e0222a6ba627ab114f0e67ee263dc5bcab540bcc69eb2f4a`.
- Installed `mrd-session-agent.exe` SHA256: `a52ff84851883b2d978e6a5cb7b5d7696cec18a1a984b07ee6bb855cda38dd06`.
- `MiniRemoteDesktop` is Running, Automatic, under the existing SCM account. The management health response reported `running=true`, `healthy=true`; real autostart readback returned enabled/supported.
- An interactive-session Agent was launched successfully. The fresh registry stays open at startup; the logon can query only the service process metadata needed to verify its parent before bootstrap.
- The ordinary user could query the restricted management pipe; a file-operation request was rejected with `E_MANAGEMENT_COMMAND_DENIED`.
- Preshutdown timeout readback is 30000 ms. Existing executable/configuration backup: `C:\Program Files\MiniRemoteDesktop\service-backup-20261003T164309Z-32056cc0`.
- Local device: `lan-LCXACE` (`LCX_ACE`). Signed LAN discovery is running on UDP21116. Ethernet IPv4: `192.168.1.136`.
- A deployment firewall rule named `MiniRemoteDesktop-LAN-UDP` allows inbound UDP only for the installed service executable and only from `LocalSubnet`. It does not open a public Internet inbound path.

## Installer corrections from actual activation

Windows PowerShell 5 stripped the embedded executable quotes and split a Program Files path in the previous native invocation. The installer now passes separate option/value arguments with explicit Windows command-line escaping through `ProcessStartInfo`, verified against an actual native argument fixture. An enum bitwise operation also needed explicit integer conversion for PowerShell 5.

The local `sc.exe` does not implement the previous preshutdown command. This setting now uses `ChangeServiceConfig2W` and verifies it with `QueryServiceConfig2W`. Failed attempts were restored to the old service; the final installation completed successfully.

Fresh verification: 12 Agent runtime tests, 16 registration tests, 3 management IPC tests, 9 shutdown tests and 4 Windows service contract tests passed. All 7 installer tests passed on both Windows PowerShell 5 and PowerShell 7, including native argument parsing and a read-only Windows service API query.

## macOS connection boundary

The Windows host is running, but a macOS-to-Windows desktop session has not been verified. The active `mrd-service` production entry point currently rejects non-Windows hosts, so a macOS controller using this same mainline needs a supported controller runtime before it can establish a session.

The installed Windows service has no `MRD_SIGNAL_URL` or provisioned device credential. It is not registered with authenticated WAN signaling merely by starting the local service. Public connection still requires device enrollment and matching protected WAN configuration; public direct-first P2P traversal remains unimplemented.

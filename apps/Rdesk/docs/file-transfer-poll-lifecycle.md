# File transfer list request ownership

`useFileTransfers` allows one in-flight list IPC request per hook instance. The next automatic poll starts three seconds after settlement. Manual refreshes during a pending request share one queued fresh read; their returned promise waits for that read, including a refresh requested during the promise settlement handoff. Disabling or unmounting clears timers and queued work. A generation check discards retired results and errors. Re-enabling waits for the old request to settle before reading the new generation.

Run the focused fake-IPC regressions from `apps/Rdesk`:

```sh
pnpm test src/app/services/fileTransferListService.lifecycle.test.ts src/app/services/ipcSessionService.test.ts src/app/services/remoteSessionStateService.test.ts --maxWorkers=1 --no-file-parallelism
pnpm type-check
pnpm build
```

The lifecycle fixture mounts the real React hook with a mocked list adapter and fake clock. A thirty-second pending query produced eleven unresolved requests in the original interval implementation; the repaired hook keeps one. Tests also cover manual refresh coalescing, settlement cadence, disabled and retired generations, late errors, rejected IPC, StrictMode replay, and saved callbacks after unmount. The original implementation passes two of the final fourteen lifecycle cases; the remaining twelve reproduce the ownership defects. A live/manual positive control and the promise handoff case preserve existing behavior.

The IPC API does not expose cancellation. A permanently pending request therefore blocks further reads for that hook, including re-enabling it. Pending native work cannot be forcibly cancelled by this change. Separate hook instances retain separate polling ownership. Tests use synthetic data and fake transport only; no remote-control, network service, capture, or input injection is started. Request counts are not native or whole-application RAM acceptance.

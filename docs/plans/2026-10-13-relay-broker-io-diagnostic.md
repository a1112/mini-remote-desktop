# Relay broker I/O diagnostic

The current browser session reached the local product pairing flow but its backend approve request returned HTTP 503. The owned relay remains registered with versions 3/3/3 and spare capacity, while its probe is failed and its lease has expired. Its socket-activated control helper repeatedly exits 70 with the single category `relay_broker_io_failed` after 54–77 ms. This rules out the helper's 14-second total timeout for those observed failures, but does not identify the original syscall.

Preserve the current identity, certificate, secret versions, audit data, socket authentication, sandbox, request deadlines and original error results. Add a private finite diagnostic to the helper's runtime initialization and existing Linux I/O error boundaries. Its stderr record contains only a fixed event tag, actual process PID, finite stage and bounded `raw_os_error` number or null. Native error text, file paths, request metadata, secrets and payloads never enter the record. Sink failure is ignored.

Cover runtime build; Linux and WSL request timeout; WSL response write/flush; socket nonblocking, registration, response write/shutdown; private artifact metadata/unlink; directory open/sync; monotonic clock syscall, invalid value and overflow. Non-OS branches use null rather than stale errno. Keep the original `IoFailed` result and exit70 in every branch. The Windows control implementation is outside this diagnostic.

Alternatives considered: broad sandbox or timeout changes lack supporting evidence; tracing request buffers would disclose unnecessary material. The fixed diagnostic makes the next ordinary daemon request sufficient to distinguish the failing boundary without either action.

Execution:

1. Write tests and retain genuine RED for finite rendering, errno bounds, rejected native text, sink failure and preserved I/O results.
2. Add the minimal diagnostic and calls, then run focused and relevant relay-agent tests. Windows tests cover portable logic, not execution of Linux syscalls.
3. Independently review the source and original branch equivalence; commit and push the reviewed change.
4. Build and verify the control-helper candidate on the Linux host from that source. Replace only the root-owned helper through a hash-bound transaction retaining an exact binary backup; preserve its owner/mode and all state/configuration.
5. Read the next ordinary daemon request's fixed diagnostic. Repair the demonstrated cause with a regression where feasible; verify three healthy heartbeats and renewed eligibility before repeating the local browser connection.
6. Require real picture and keyboard/pointer evidence before running 30/60/120 requested profiles and CPU/GPU measurements.

The user has authorized continued fixes, updates and automatic tests without repeated permission requests. No diagnosis or candidate build is a substitute for video/input acceptance.

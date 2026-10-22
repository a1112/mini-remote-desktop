# TLS-compatible coturn production installer design and implementation plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Install the qualified coturn candidate through one bounded maintenance operation while preserving registered identity, certificate, credentials, configuration, registry counters and normal broker ownership.

**Architecture:** A new fixed maintenance owner takes authority after the existing certificate owner completes its formal transition and exits. It captures fresh kernel/file/systemd/registry facts, holds the original process and file objects, performs one atomic executable replacement, and retains the new generation through acceptance or bounded recovery. Historical JSON is evidence; it never recreates a held process or authorizes a production mutation.

**Tech Stack:** Linux Python 3, systemd typed D-Bus properties, pidfd/cgroup/procfs, SHA-256, flock, secure file descriptors and atomic rename; coturn/OpenSSL/prometheus; existing mrd-relay-agent broker and registry API.

**Status:** DESIGN ONLY. No installer was implemented or production command executed for this plan. Isolated compatibility qualification is complete; production eligibility remains false until the runtime, certificate and ownership gates below are collected and reviewed. The original 8dda/e33/52/02b packages and their failure/preservation records stay immutable.

## Frozen qualification and candidate

- Candidate: /root/mrd-coturn-tls-compat-7a82a198-324c-46e6-8375-712dcfb1a26d/build-compat/bin/turnserver; 706128 bytes; SHA-256 4fc6d1cb6f5035b99a35ff6b84295a241a5715d4f44396dc1acdca01aff84546; version 4.17.2; official source commit de0c9b28f22281a0251d7e98aa8a097895a5b185.
- Allocation patch: 9d5d75e01088952e8f065a890d1e8410ef1af60d25e9806c3e6a8d70648e6bcb. Additional TLS compatibility patch: 06dca1e857c75e2da080b2211c95bcabbca10a7734ed29077cf50cc2fbc87b8c. Only mainrelay.c changed for TLS; mainrelay.h ABI remains 8ba4848ff3afd716c7fb8e1b77f56faebb4863ebc335f250d48fa08495755513.
- Original qualified mainrelay.c: 6be65267500301971e395180cc8846e3333a853e17b65cb02f729d72679b571e. The old 02b candidate remains unchanged and is not the install target.
- Qualification supplement: L:/project/mini-remote-desktop/target/browser-remote-debug/coturn-tls-compat-supplement-20261010-be6899b7-af34-4990-9d1b-65aaf8b10653/qualification-supplement-manifest.json, SHA-256 75032481acb010097ba98e615898a6b4d166074fa2c1f2b00b31aa487af8d9f4; seal 215ea865a118b8e103482e4bce16583a76e9cf5464375c4e2f548abb707d635c. Independent complete audit SHA-256 e83881c0d2910773fe4b726c41b00f633b3d0f1ba41b2120cb8aca9e3dbc88e7.
- Actual checks: full CMake configure/build 0/0; 10 option matrices and 120 real cold/reload protocol probes plus 40 real context publication probes; four cold and four reload failure injections (TLS min, DTLS min/max and NULL context); typed cold metrics and nine authenticated UDP/TCP/TLS Allocate/Refresh600/Refresh0 integrity-verified events with release counts checked before closing sockets.
- Both broker-required bare legacy flags reject TLS1.0/1.1 and DTLS1.0 by real server protocol-version alerts while TLS1.2/1.3 and DTLS1.2 work. Absent/false flags preserve default floor0. Existing DTLS MAX1.0 semantics remain: strict1.2 clients reject actual ServerHello BODY1.0, independently parsed from all 14 original public datagrams. This MAX case is not used to relax legacy MIN rejection.
- Failure observer forwards the real setter and injects reported return0; tests prove application fail-closed handling. Historical early SIGUSR2 fixture failures remain preserved and were resolved by proving caught-mask and both context publication before signal.

## Options and selected maintenance ownership

1. **Recommended: new binary-only owner after explicit certificate-worker handoff.** Keep the frozen certificate worker intact; obtain its actual committed response and normal exit, then acquire the same node lock and recapture all current authority. This preserves reviewed certificate behavior and limits new code to binary installation/recovery. The lock gap is closed by fresh authoritative gates, not a stale receipt.
2. **New combined certificate-plus-binary worker.** This could retain one lock throughout the operation but would require a new formal-enrollment controller, substantially more tests and a separate complete review. The frozen worker cannot simply be edited because its original executable/file pins intentionally forbid replacement.

Use option 1. Do not reconstruct old nonce/epoch/sequence/version authority from the prepared 8dda plan. Its historical original binary SHA and old identity values are comparison references until a fresh collector confirms the exact current baseline.

## Task 1: Complete and review runtime applicability before outage

Future files, all in a new private installer nonce: applicability-collector.py, applicability-policy.py and tests/test_applicability.py. Do not overwrite the frozen config-owner or 8dda collector.

1. Read the full existing read-only collector and bind its SHA. Query typed systemd D-Bus properties for mrd-coturn.service: Type, ExecStart, Environment, EnvironmentFiles, NotifyAccess, WatchdogUSec, Restart, RestartUSec and cgroup/invocation information. Preserve types and method errors. Missing output, an unsupported property, failed D-Bus read or flattened empty systemctl output is not evidence of an empty EnvironmentFiles array.
2. Store only nonsecret metadata and full protected evidence on the server. EnvironmentFiles paths are not file contents. If typed files exist, pin their canonical paths/metadata/SHA and determine effective environment through a separately reviewed protected reader; never print environment values or credentials. Preserve loader-affecting settings, do not erase them to force compatibility.
3. Validate candidate dependencies under the actual protected service environment. The earlier 02b library-resolution supplement does not prove the new 4fc candidate's resolution. Record exact ELF interpreter, feature requirements, resolved loader/libraries and their inode/SHA metadata. The candidate has no systemd notify support; Type and watchdog expectations must be compatible. Required configuration must not rely on features absent from the build.
4. Verify every broker-generated directive is recognized and preserves effective behavior. Pin broker.rs SHA ba16e9754ac4e6fafd85c48540072be4948a28faedaccfe60a847ef50503b843 and the current protected generated configuration SHA. No configuration rewrite is included.
5. Pure fail-closed fixtures: absent-vs-empty typed EnvironmentFiles, DBus failure, nonempty effective files, notify/watchdog incompatibility, unsupported feature/directive and library/metadata drift. A passing collector only authorizes preparing the installer; it does not stop or install anything.

## Task 2: Freeze concrete current authority and recovery before outage

Future files in the new installer nonce: fixed-plan.json, owner.py, recovery.py, integrity.py, tests/test_owner_policy.py and private server evidence directory.

1. Resolve current actual certificate-owner state and formal registry transition using the existing reviewed enrollment/approval/pickup boundary. Require current client identity/certificate validity, the same registered node/key, owned receipt/request and no private-key replacement. The old expired identity remains audit material and must never be restored over a newly valid identity.
2. If the certificate worker is the authorized stopper, it retains its original pidfds/file pins until its actual committed result. Obtain one idempotent fresh commit response, then normal cancel/exit0 on that same channel. Only after actual exit may the new owner acquire /root/.mrd-relay-maintenance/relay-tencent-gz-1.lock. A lost worker is an unknown outcome, not a replayable handoff.
3. Pin exact current installed ELF, candidate ELF, unit/drop-ins/environment/config, TLS/CA, identity/runtime/broker/secret metadata and registry state. Never assume the historical epoch102, sequence4, secret1/1/2 or original PID remains authoritative. Fresh plan values are concrete and reviewed; no wildcard hashes or arbitrary caller-selected paths/roles are allowed.
4. Require no active allocation, unexpired reservation or related nonterminal session in the authoritative registry. Require valid typed zero metrics before a live stop when the current exporter supports them; missing historical exporter samples do not become zero. Use its separately reviewed alternative drain proof only if explicitly included in the fresh plan. No SQL count clear or counter reset is allowed.
5. Build and seal recovery first. Secure root0700 directories and root0600 evidence/backup files using exclusive, nofollow creation; hold original installed executable and immutable candidate file descriptors; copy backup bytes and exact owner/group/mode/ACL/xattrs with fsync and hashes. Do not broaden permissions, reset service budgets or restore protected state snapshots.

## Task 3: Stop and install with held generation proof

1. Before any normal STOP, open pidfds for the exact current coturn generation and verify PID/start/executable inode+SHA/argv/config/cgroup/InvocationID. Retain the kernel objects. Record the complete raw control-command outcome. A STOP nonzero does not prove the process survived or exited; the held object and fresh service facts decide.
2. Verify original pidfds exited normally, no replacement invocation, full recursive cgroup empty, fixed listeners down and global TURN/selected unknown candidates absent. Require at least16 seconds of fresh stable stop proof. If the unit has automatic restart actions, use their actual maximum delay and exhausted budget in the concrete plan; 16 seconds alone cannot defeat a pending restart.
3. Revalidate certificate/identity, configuration/environment and registry/current session/count CAS under the node lock. Any drift blocks. No executable replacement occurs while a new or unheld coturn generation may run.
4. Prepare one root-owned sibling under /usr/bin from the held candidate bytes, exact expected metadata, fsync, rehash and recheck all gates. Atomically replace /usr/bin/turnserver once and fsync the directory. Verify installed content and inode. No package-manager install, daemon-reload, unit/drop-in/environment rewrite, certificate/key copy or unrelated executable replacement is included.
5. Append actual checkpoints before and after each mutation. Interrupted/unknown rename or command outcomes never cause automatic replay. Recovery uses actual completed steps and fresh kernel/SCM-equivalent systemd facts.

## Task 4: One normal start, acceptance and bounded recovery

1. Select the start route from the actual broker/unit ownership contract. Direct systemctl start must not silently create a generation inconsistent with the broker; a new authorized managed transition must use the normal existing interface and preserve its monotonic counters. Pin one reviewed operation and exact arguments.
2. Before START, launch a bounded generation watcher. Once a candidate MainPID appears, verify its full tuple and immediately hold its pidfd through acceptance/recovery. A positive PID/InvocationID observed even briefly is a real generation. START nonzero cannot be treated as never-started if such a generation appeared.
3. Require candidate-only installed/executing SHA, stable expected listeners, unit health and valid typed cold allocation0 while the registry stays quiet. Do not authenticate a production TURN allocation as installer preflight. Real guest/Mac/WAN acceptance follows the separate product-session workflow once the relay is formally active; isolated TURN tests do not prove WAN delivery.
4. Preserve current valid identity/TLS/CA/runtime/broker/secret and registry counters. Any agent start, signed heartbeat/lease publication or desired-secret rotation is its own explicitly bound normal transition with actual acceptance, never a synthesized JWT or direct SQL mutation.
5. Recovery before START: only with continuously quiet inactive/count0 plus exact CAS may the held verified old ELF be restored atomically. Keep the newly valid identity and every nonbinary artifact intact.
6. Recovery after an observed generation: one normal stop while its exact pidfd is held, normal exit plus no new invocation/cgroup/listener/global process, stable quiet proof, then old-ELF restoration. A missed pidfd, forced kill, timeout or unknown generation blocks automatic rename. A never-created-generation branch requires all start observations to show no PID/running/invocation and separately reviewed quiet proof; it cannot be inferred from a nonzero command status.
7. One reviewed old-service recovery start may be included with real health verification. No unbounded retry loop, restart-budget edit, rollback to expired certificates, secret/version rewind, identity reset or unrelated process termination is included.

## Task 5: Meaningful isolated fixtures and final execution handoff

1. Fixtures use only new own loopback processes and temporary files: wrong candidate/source hash; symlink/inode/config/environment/lease/session/count drift; prior certificate owner still live or lock occupied; generation replaced; STOP error with actual held exit; START error with observed new generation; normal rollback, no-generation rollback, timeout/kill/unknown-outcome rejection; backup, fsync and rename failures; exact metadata preservation.
2. Test fail-closed decision logic with realistic differing outcomes. Do not mirror implementation branches with tautological assertions. Reuse successful TLS/metrics qualification receipts; do not rebuild or rerun the 120+40/9 tests without a source or unresolved-runtime change.
3. Final package includes exact current plan and bounded TTL, the fixed owner/recovery/collector source and hashes, immutable candidate provenance, pure/native fixture outcomes, independent source review and one-shot launcher. Root independently verifies all hashes and gates before dispatch. This design document is not executable authority.

## Acceptance and remaining gates

The completed isolated qualification may be relied on only for the fixed 4fc candidate. Typed production EnvironmentFiles/effective environment, actual new-candidate library/runtime/config applicability, exact certificate-worker handoff and fresh idle/drain/current-generation authority remain to be established. The installer code, recovery fixtures and seal still need implementation and independent review. Until then production_eligible=false and no production STOP/install/start is performed by this task.

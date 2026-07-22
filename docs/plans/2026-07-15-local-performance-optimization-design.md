# Local Performance Optimization Design

## Scope

Optimize every performance test that is executable on this Windows workstation:

- component matrix;
- all Quick, Steady, and Stress transport scenarios;
- automated synthetic end-to-end matrices;
- local dual-process canary profiles.

Tests that require a second physical peer or a real TURN service are excluded.
Locally unsupported codec paths remain explicit unsupported/infrastructure
outcomes and may not be converted into product passes.

## Workstation Baseline

- Intel Core i5-14600KF, 14 cores / 20 logical processors;
- NVIDIA GeForce RTX 5060 Ti, 16 GiB, driver 620.02;
- 64 GiB RAM;
- Windows 11 Pro for Workstations Insider build 26300;
- one active monitor plus installed virtual-display drivers;
- background remote-desktop applications remain running and are recorded as
  environmental noise rather than terminated.

The initial component matrix passed all 14 rows. The closest performance margins
are DXGI capture, OpenH264, and NVENC tail latency; transport and D3D11 rendering
have substantial headroom.

## Method

Use an evidence-driven, tiered loop:

1. Validate harness contracts.
2. Record environment, warm up the selected path, and run scenarios serially in
   release mode.
3. Classify every row as PASS, PRODUCT_FAIL, INFRA_FAIL, INVALID_ARTIFACT, or
   unsupported with a stable reason.
4. Optimize actual failing rows first, followed by rows with the lowest threshold
   margin.
5. For every code change, add a deterministic behavioral or regression test
   before implementation, then repeat the affected performance scenario at least
   three times and compare medians.
6. Re-run the complete local suite, including Steady, Stress, and dual-process
   canaries.

Thresholds are changed only when the measurement contract is demonstrably wrong.
They are never relaxed to hide a product regression.

## Reproducibility

The local suite writes:

- an environment manifest with hardware, OS, driver, commit, and dirty state;
- one attempt record per component/scenario;
- links to native component and benchmark artifacts;
- an aggregate JSON and Markdown report;
- failure classification and rerun history;
- before/after median comparisons for optimized rows.

Runs are invalidated when the harness crashes, artifacts are missing, cleanup
fails, or required metrics are absent. Background load is sampled and recorded;
the suite does not mutate user power settings or terminate unrelated processes.

## Optimization Boundaries

Product optimization may change capture scheduling/copy behavior, encoder
configuration and buffer reuse, decoder queues, transport batching, or render
pacing when evidence identifies those paths. It must preserve output validity,
codec negotiation, security boundaries, frame ordering, cleanup, and visual
quality contracts.

## Completion Criteria

- all locally supported component rows pass;
- all locally supported Quick scenarios pass their existing thresholds;
- Steady and Stress scenarios pass without missing metrics or cleanup failures;
- automated E2E and local dual-process canaries pass;
- every unsupported row has an honest stable classification;
- the full rerun produces a complete aggregate report;
- formatting, focused tests, and affected crate tests pass.

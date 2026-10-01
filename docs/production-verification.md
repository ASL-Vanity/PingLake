# PingLake production verification record

Date: 2026-10-02

Branch: `codex/pinglake-monitoring-production`

The monitoring implementation is committed and the local Windows verification
has passed. This record distinguishes automated evidence from checks that need
an actual HTTPS deployment, a Linux host, or a Windows service account.

## Passed on the local Windows host

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --locked -- -D warnings`
- `cargo test --workspace --locked`
- `cargo build --workspace --release --locked`
- `npm --prefix web test` — 23 tests passed
- `npm --prefix web run build`
- `scripts/e2e-smoke.ps1 -Port 18090 -LeaveRunning`
- `scripts/e2e-monitoring.ps1 -RunRoot <smoke-run-root>`

The monitoring E2E run verified six controlled probes, two service checks,
one process check, one local-port check and canonical DNS statistics,
configuration revision application, detailed history with collection and Hub
receipt timestamps, probe percentile statistics, group deletion preservation,
the latency endpoint's allowed and rejected origins, and the no-store response.
The smoke run verified seven registered nodes, seven online nodes, history
points, authentication, offline detection and an offline alert.

The Agent test suite covers the persisted pending-report queue, queue overflow,
retry jitter, v1 schema downgrade, continued collection during Hub failures,
DNS/ICMP/TCP/HTTP policy behavior, process checks, local socket observations,
Windows SCM fixtures and native Windows collector fixtures. The Hub suite covers
schema v6 canonical check storage and schema v5 migration,
deduplication, alert recovery, statistics and authorization. Protocol tests
cover legacy enrollment defaults, strict configuration decoding and v2
fixtures.

The latest full Rust run completed 52 Agent tests, 26 Hub tests and 10 Protocol
tests. Static analysis passed with `cargo clippy --workspace --all-targets
--locked -- -D warnings`.

## Compatibility behavior

The Hub advertises monitoring schema v2 during enrollment. A new Agent talking
to an older Hub defaults to v1 and removes v2-only probe results and
configuration before upload. The Agent configuration request includes its
maximum schema; the Hub returns a v1-compatible configuration to an older
Agent. Existing v1 reports remain accepted by the current Hub.

The Agent's pending queue is bounded at 64 reports and persisted below its
state directory. Reports older than 300 seconds are expired. A corrupted or
oversized queue is reported and ignored so it cannot prevent the Agent from
starting.

## Environment checks still required before production rollout

- Open the built Web console in a browser and verify desktop and mobile layout,
  login, navigation, host details, configuration editing, service/process/port
  tables and responsive overflow. The in-app browser could not obtain a
  permission decision for the local HTTP origin during this run, so no browser
  visual result is claimed here.
- Run the browser-to-node latency test through real HTTPS reverse proxies on at
  least two monitored nodes. Confirm TLS, CORS, CSP, cache headers, timeout and
  origin rejection.
- Run the Agent as Windows `LocalService` and record process, SCM, IP Helper,
  PDH and ICMP visibility. Administrator runs do not substitute for this check.
- Build and run the Linux Agent as the installed `pinglake` system user,
  including systemd present/absent, ping socket policy, procfs visibility and
  container network namespaces.
- Start Docker before running the Linux Agent image build and Hub Compose
  checks. The Docker daemon was unavailable during this verification.
- The installed Windows Rust toolchain has the Linux target, but the Linux
  cross-check could not build `ring` because `x86_64-linux-gnu-gcc` is absent.
  The available WSL distribution did not have Rust/Cargo installed, so it could
  not substitute for the missing Linux build environment during this run.
- Perform a real SQLite backup including WAL, v4-to-v5 migration rehearsal,
  restore rehearsal, Hub-first upgrade, staged Agent upgrade and rollback.

No production DNS, firewall, service installation, database, or external node
was modified by this verification run. The temporary local E2E Hub and Agents
were stopped after testing.

# PingLake monitoring development plan

Date: 2026-10-01
Status: implementation and automated verification complete; browser visual and deployment checks remain.

## Confirmed scope

Implement the following ten monitoring items, group deletion in the Web UI,
and browser-to-each-node latency. This plan covers development and local/lab
verification. Production installation, firewall changes, DNS changes and
service restarts are a separate deployment step.

| Item | Deliverable |
| --- | --- |
| 1. Per-core CPU | Utilization for each logical CPU, frequency when available; Linux user/system/IOWait/steal breakdown |
| 2. Memory and swap | Available memory, cache/buffers where meaningful, swap usage and swap-in/out counters and rates |
| 3. Disk IO | Per-device read/write bytes/s, read/write IOPS, IO latency, utilization/queue metrics where available; host CPU IOWait shown separately |
| 4. Inodes | Per-filesystem total/used/free inodes and usage percentage on Linux |
| 5. Network health | Per-interface bytes, packets, errors, discards/drops, cumulative counters and rates; interface state and identity |
| 6. TCP connections | Counts by TCP state, IPv4/IPv6 and listening sockets; explicit visibility limitations |
| 7. Specified services | Read-only status of configured systemd units and Windows services, native state and normalized health |
| 8. Agent quality | Collection/send duration, attempted/successful/failed uploads, rolling success rate, retries, consecutive failures, last success, bounded-buffer drops and classified errors |
| 9. Active probes | Configurable ICMP, TCP and HTTP(S) checks from specified Agents, timeout/interval/concurrency limits, result history |
| 10. Latency and availability | Probe P50/P95/P99, success rate, failure count and monitoring coverage for explicit time windows |

GPU, SMART, container metrics, application/database instrumentation, RBAC,
Prometheus, enterprise alert workflows and long-term storage are outside this
iteration. Existing process Top metrics and basic alerts remain supported.

## Verified baseline

- Workspace consists of protocol, Agent, Hub and React Web modules.
- Agent reports every five seconds by default and uses sysinfo 0.37.
- Upload backoff currently blocks further collection; scheduling must change.
- SQLite schema version is 4; raw metrics are retained for seven days.
- Existing history returns at most 1,440 points after loading matching rows.
- History currently labels Hub receipt time as collected_at. Keep query windows
  based on Hub receipt time, but expose collection and receipt times distinctly.
- DELETE /api/v1/groups/{id} and web/api.ts deleteGroup already exist.
- Deleting a group uses ON DELETE SET NULL: nodes become ungrouped; nodes and
  monitoring history remain. The handler currently emits no group SSE event.
- Card/detail latency currently reads Agent-to-Hub report request duration.
- Hub Content-Security-Policy currently limits connect-src to 'self'.
- Linux installer uses an unprivileged service account; Windows uses LocalService.

## Browser-to-node latency

The user confirmed the path is browser -> each monitored host, not browser -> Hub.

- Each node has an administrator-configured, independently reachable HTTPS
  measurement URL. No address is inferred from hostname or enrollment data.
- Browser issues a small non-cached request and measures elapsed time using
  performance.now(). Display it as HTTP application round-trip duration, not
  ICMP latency or one-way latency.
- Provide an optional minimal read-only node measurement endpoint and deployment
  instructions for HTTPS termination through an existing reverse proxy. Listener
  is disabled by default; no port or firewall is opened during normal enrollment.
- Endpoint exposes no metrics, credentials or commands; responses use no-store.
  Apply request limits and precise CORS rules for the PingLake website origin.
- Hub CSP permits only explicitly configured measurement origins, validated as
  HTTPS origins. Do not broaden connect-src to all URLs.
- Validate URL scheme, embedded credentials, size and allowed network exposure.
  Private-address targets require an explicit policy and may be inaccessible
  because of browser local-network permission, mixed-content or CORS rules.
- Browser uses bounded parallelism, request timeout, cancellation on navigation,
  and pauses periodic work in hidden tabs. Cache readings only in that browser
  session and invalidate them when a node endpoint changes.
- Show unconfigured, measuring, stale and unavailable states. Browser fetch
  errors cannot always distinguish CORS, TLS and network failure; do not invent
  a specific diagnosis or use zero milliseconds as a failure value.
- Agent-to-Hub duration remains available in the diagnostic view with its true
  label. Hub relaying is never presented as browser-to-node latency.
- Browser observations are session-specific and initially remain local to that
  session. Persisted availability/P95/P99 come from authenticated Agent probes,
  with the probe source shown; browser results do not set global server health.

## Metric contract and platform support

Freeze the Rust and TypeScript contract before parallel implementation.
Use compatible optional fields/substructures with serde defaults for old Agents,
stored JSON and UI responses. A legacy report must remain valid.

- Metrics have documented units, source, collection time and capability/status.
  Distinguish ok, warming_up, unsupported, permission_denied, unavailable and stale.
- Optional unavailable values are null, never fabricated zero values.
- Use stable device identifiers where available plus human-readable names.
  Document identity limitations for hotplug, renaming and virtual filesystems.
- Preserve raw counters and derive rates using monotonic elapsed time. Counter
  resets, first sample and device replacement invalidate a rate interval.
- JSON must not transmit u64 counters above the JavaScript exact-integer range
  as ordinary numbers; encode large cumulative counters as decimal strings.
- Collection time and received_at are separate. Clock skew is not interpreted
  as exact network transmission time. Freshness/coverage use Hub receipt time
  and expected report intervals, with Agent sample age measured monotonically.
- Collection/session ID and sample sequence allow ingestion to deduplicate
  retries without losing legitimate samples after Agent restarts.

| Area | Linux | Windows |
| --- | --- | --- |
| Per-core CPU | sysinfo; /proc/stat for CPU state deltas | sysinfo; native counters for supported CPU breakdown |
| Available memory | sysinfo; /proc/meminfo for details | sysinfo/native memory counters; cache semantics labeled explicitly |
| Swap IO | /proc/vmstat counters, page-size conversion | Pagefile usage; native paging counters when available, never label all paging as swap IO |
| Disk IO | /proc/diskstats and stable block-device identifiers | PDH/native disk counters and stable device mapping |
| Inodes | statvfs-compatible filesystem calls | Not applicable |
| Network | sysinfo plus native/sysfs counters | IP Helper/native interface counters |
| TCP state | /proc or native socket diagnostics; visibility status | IP Helper IPv4/IPv6 TCP tables |
| Services | Read-only systemd status for allowlisted units | SCM queries for allowlisted service names |
| ICMP | Proven ICMP library/native API; permission capability check | Windows ICMP API/library adapter |

IOWait is a CPU time metric; it is distinct from disk request latency. Steal and
inode have no fabricated Windows equivalents. Disk devices and mount points are
different dimensions; do not double-count partitions, bind mounts or virtual
devices in totals. TCP state counts do not claim to measure all processes' sockets
when the service account or network namespace limits visibility.

## Architecture and storage

- Prefer existing sysinfo/Tokio/reqwest and structured OS APIs. Evaluate an
  established ICMP implementation; do not hand-write the protocol engine.
- Separate host collection, service checks, active probes and upload workers.
  Slow or failing probes cannot block five-second host collection.
- Use a bounded upload queue with a documented drop policy and drop counters;
  do not introduce unlimited offline buffering. Failed reports retain original
  timestamps and identities; successful receipt does not make old data fresh.
- Counters describing the current upload outcome appear in a subsequent report.
  While offline, Hub derives missing reports from receipts; Agent local failure
  counters become observable after reconnection, not magically during outage.
- Slow collectors and native blocking APIs run off the asynchronous executor.
  Limits, deadlines and shutdown cancellation apply to every collector.
- Typed, versioned service/probe configuration is persisted by Hub and retrieved
  through Agent-authenticated configuration APIs. Local policy determines which
  service names and probe targets that Agent may inspect. No remote shell commands.
- Track saved configuration revision and Agent-applied revision separately.
  UI shows pending, applied or application failure; Hub save alone is not success.
- Start with at most 32 services and 32 probe targets per node, four concurrent
  probes, a 30-second default probe interval and timeouts shorter than the interval.
  Bound report size by the existing 256 KiB ingestion limit; detailed history
  selects devices/series with a response byte limit as well as 1,440 time buckets.
- Store optional host-detail JSON alongside current scalar metrics; create
  indexed service/probe result tables for query and aggregate statistics.
- Add validated API contracts for node measurement URL, service/probe management,
  detailed history and statistics. Enforce bounded list sizes, body sizes, finite
  values, authorization and node ownership. Redact secrets in target/error output.
- Probe redirects and DNS resolution must recheck target policy; allow intentional
  private-network checks through explicit Agent policy, while denying unexpected
  loopback/link-local/metadata targets. Disable redirects by default.
- HTTP checks validate TLS, expected status and optional bounded response matching.
  TCP success means connection establishment, ICMP success means a valid reply;
  these are distinct probe semantics.
- Existing seven-day retention applies to new raw results. Use indexed bounded
  queries and server-side time buckets for detailed historical responses.
- Online migration adds compatible columns/tables. Validate migration on a copy
  of a version-4 database and keep a pre-migration backup. Rolling back to an old
  binary after a write migration requires restoring the compatible backup.
- Use SQLite backup/checkpoint-aware procedures; copying only the database file
  while ignoring a live WAL is not a valid rollback backup.
- Service/probe failure alerts reuse existing notifications and include subject
  IDs in deduplication. Missing or unsupported data must not resolve an alert
  by being treated as a healthy zero-valued sample.

## Statistics

- Compute P50/P95/P99 from successful raw probe durations in a named time window
  using a documented nearest-rank algorithm. Never average bucket percentiles.
- Show successful sample count and insufficient-sample states, particularly for
  P99. Failed/timeout observations count as failures, not zero-duration samples.
- Attempt success rate = successful checks / completed valid attempts. Missing,
  disabled, cancelled or permission-limited checks are separate states.
- Coverage compares observed checks against scheduled checks while enabled,
  accounting for configuration changes. Show Agent-offline/unobserved intervals
  distinctly, not as healthy intervals.
- Time-based availability uses scheduled intervals and explicit unknown duration;
  do not label sample success rate as uptime or SLA. Show denominators and sources.
- Deduplicate retry uploads before statistics. Device histories keep gaps; do not
  average missing metrics into zero or interpolate outages into healthy periods.
- Aggregate disk latency using completed-operation counts as weights. Keep ICMP
  packet loss separate from probe-run failure rate, interface discards and upload
  failure rate. Each denominator is named explicitly.

## Development phases and ownership

1. Contract and fixtures (lead agent)
   - Freeze optional protocol, platform capability matrix, measurement semantics,
     configuration schema, sample identity, API responses and fixture examples.
   - Review queue policy, target policy and database migration before delegation.
2. Three parallel implementation tracks
   - System collector agent: new platform collector modules, metrics.rs and
     collector tests. Implements items 1-6 and service status collection.
   - Hub agent: db.rs, migrations, configuration/ingestion/statistics handlers,
     SSE group changes, history and backend tests.
   - Web agent: types.ts synchronized to contract, NodeTable/group controls,
     detail tabs/history, probe/service configuration, browser latency hook and UI.
   - Lead: shared protocol, Agent runtime/client/config scheduling, ICMP/TCP/HTTP
     engine, typed remote configuration, dependencies and node endpoint packaging.
   - Shared files have one owner; changes to contracts go through the lead first.
3. Integration (lead)
   - Integrate all tracks, fix contract/ownership conflicts, update installers
     and documentation, verify legacy Agent and existing data behavior.
4. Verification and handoff (lead with focused peer review)
   - Complete both platform checks, end-to-end and browser testing; report
     supported/unsupported metrics and evidence. Prepare a deployment/rollback
     checklist separately from development completion.

## Acceptance gates

- Group deletion: empty/populated groups, confirmation/cancel, duplicate request,
  missing group, request failure, empty node inventory, filter reset and two-browser
  synchronization. Nodes, credentials and history survive group deletion.
- Browser latency: two real node endpoints produce independently measured RTTs;
  no Hub fallback. Verify TLS/CORS/CSP, caching, timeout, endpoint changes,
  unconfigured/offline status, hidden tabs and cancellation on desktop/mobile.
- Host collectors: Linux/Windows deterministic counter fixtures plus actual
  service-account runs; first sample, resets, hotplug, namespaces, permission
  denial and OS-specific unavailable metrics. Validate against OS tools.
- Scheduling/quality: simulated Hub outage does not stop collection; bounded
  queue, retries, overflow counters, sample deduplication, backfill freshness,
  recovery, credentials rejection and shutdown are checked.
- Probes/services: controlled ICMP/TCP/HTTP success/failure/timeout targets and
  controlled services; HTTP TLS validation/redirect policy, IPv4/IPv6, disabled
  configuration, update/delete and restart persistence. No shell injection.
- Database/API: version-4 migration and rollback rehearsal on copies, legacy
  report ingestion, unsupported metrics, two timestamps, device history isolation,
  response limits, retention cleanup and authorization.
- Statistics: deterministic known P95/P99 values, retry duplicates, zero samples,
  timeout, outage gaps, schedule edits and coverage/time-based availability.
- UI: build/typecheck, protocol fixtures for old/new Agents, desktop/mobile
  screenshots, overflow checks, keyboard access and real edit/delete flows.
- Regression: cargo fmt/check/test, front-end build, existing e2e smoke plus the
  new workflows, Linux and Windows Agent packaging and installer checks.
- Resource check: benchmark default collection at five seconds with representative
  host/device counts and offline queue pressure; establish a baseline and record
  CPU/RAM/report size/database growth before declaring the change ready.

## Current progress

- [x] Scope confirmed: ten monitoring items, group deletion, browser-to-each-node latency.
- [x] Read-only repository and platform/design assessment by three subagents.
- [x] Development phases, responsibilities and acceptance gates prepared.
- [x] Protocol/configuration contract frozen; compatible optional metrics and typed target configuration added.
- [x] Implementation and automated local/lab verification.
- [ ] Browser desktop/mobile visual and interaction checks (browser permission verification unavailable).
- [ ] Real HTTPS browser-to-node measurement and Windows LocalService permission verification.
- [ ] Deployment preparation and production rollout, when separately authorized.

# Monitoring validation record

Date: 2026-10-01 (Asia/Shanghai)

## Result

The ten monitoring additions, Web group deletion and browser-to-node latency
implementation are complete. Automated checks and controlled endpoint integration
passed. This is a development result, not confirmation of a production rollout.

| Check | Result |
| --- | --- |
| Windows workspace tests | 69 passed: Agent 42, Hub 25, protocol 2 |
| Linux Agent native Docker tests | 44 passed |
| Linux uid65534, no capabilities, no-new-privileges | 44 passed |
| WSL ordinary-user systemd checks | Running dbus.service and missing unit verified |
| Web measurement tests | 4 passed |
| Web TypeScript and production build | Passed |
| cargo fmt check | Passed |
| workspace Clippy, all targets, warnings denied | Passed |
| Windows optimized Hub and Agent build | Passed |
| Linux x86_64 musl optimized Agent build | Passed |
| Linux release startup without privileges/network | Version command passed |
| Existing seven-node smoke | Seven enrolled/online, persisted history |
| Extended monitoring smoke | Five probes, two services, applied configuration, history and percentiles passed |
| Group deletion integration | Node and metrics preserved, moved to ungrouped |
| Local measurement handler | Empty 204/no-store, expected Origin accepted, foreign Origin rejected |

Automated tests cover old Agent reports and v4 database migration, future schema
rejection, scoped authentication/configuration, transactional revision conflicts,
device counters/reset, unavailable metrics, per-subject alarms/unknown data,
retry deduplication, original timestamps, retention, fair scheduling, bounded
buffers, oversized-report fallback and optional endpoint failure isolation.

Statistics tests cover nearest-rank quantiles, latency sample counts, versioned
intervals, paused/missing samples, repeated schedule slots and target definition
changes without mixing unrelated latency distributions.

Frontend tests use controlled fetch/time/stream fixtures to verify independent
endpoint durations, omitted credentials, no-store, forbidden redirects, failure
instead of zero latency, cancellation and a 64 KiB response limit. They do not
substitute for real HTTPS browser measurements or layout screenshots.

## Controlled integration run

The test Hub binds only 127.0.0.1:18090. Seven temporary Agents report local
Windows metrics. One Agent checks loopback ICMP, TCP, HTTP, a closed TCP port,
a policy-denied metadata address, EventLog and a nonexistent service.
Only the local test Agent enables loopback probing and the optional measurement
listener. The metadata target is denied before any connection.

Representative measured resource use with seven test Agents:

- Mean Agent working set: 31.44 MiB.
- Mean serialized report: 47.14 KiB; largest observed report: 64.25 KiB.
- Hub working set: 22.96 MiB.

These are local measurements, not a Linux baseline or a large-fleet benchmark.
Test-run state and random test credentials are under ignored data/e2e-* directories.
No real production credentials appear in this record.

## Remaining environment checks

- Browser automation was attempted twice but navigation was blocked because
  saved browser permissions could not be verified. No security-control bypass
  was used. Desktop/mobile screenshots and actual page workflows remain unverified.
- Two real, independently reachable HTTPS node measurement endpoints have not
  been deployed. Native HTTP/CORS handling and isolated browser request semantics
  were tested; actual visitor-to-node TLS/CORS/CSP paths remain to be checked.
- Windows native collectors passed under the current account. The process is
  not elevated, so a LocalService test service was not installed. Its actual
  permissions remain an explicit validation gap.
- Production Hub/Agents, DNS, firewalls and service configurations were not changed.
- Disabled/deleted failing checks retain active incident records; manual incident
  cancellation/acknowledgement is outside this iteration.

See monitoring-guide.md for platform differences, setup, metric definitions,
buffer limits, database backup/rollback and reproducible test commands.

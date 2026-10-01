# Monitoring extension

The Hub remains compatible with original v1 Agents. New Agents add per-core
CPU, available memory/swap IO, disk IO, inodes, interface errors/discards, TCP
states, service checks, upload quality and ICMP/TCP/HTTP probes. Detailed
history and probe statistics are available in the node detail tabs.

## Platform semantics

- Linux CPU IOWait is CPU time spent waiting; disk read/write latency is the
  completed-request average. These metrics are not interchangeable.
- Disk IO is per block device. Inodes and capacity are per mount point.
  Logical volumes, partitions and underlying devices must not be summed twice.
- Windows supports native PDH disk IO and CPU user/system counters, IP Helper
  interface/TCP counters and SCM service status. Linux inodes, CPU steal/IOWait
  and exact swap-in/out have no fabricated Windows equivalent.
- Linux TCP describes the Agent's current network namespace. A container's
  socket table does not necessarily describe its host.
- Interface discards/errors are separate from ICMP packet loss or probe failure.
- First rate sample and counter reset return a warming-up/unknown value.
  Unsupported, permission-denied and unavailable values are not zero.
- Raw cumulative resource counters are decimal strings, preserving u64 precision.

## Service and probe configuration

Use the node's Configuration tab to create or edit checks. Service names are
systemd unit names on Linux and SCM service names on Windows, not display names
or shell commands. Expected states are running or stopped.

Probe kinds have distinct meanings:

- ICMP: a single echo request/reply round trip per scheduled check.
- TCP: connection establishment to the configured host and port.
- HTTP(S): time to response headers, or to the bounded response body when content
  matching is configured. Default expected status is 2xx; TLS validation stays on.

Each node permits at most 32 services and 32 probes. Probe intervals are
10-86,400 seconds, with a default of 30 seconds. Timeouts are 1-10,000 ms and
shorter than the interval. Four probes run concurrently, with fair scheduling.
Excess demand is visible as reduced coverage rather than unlimited task buildup.

The Hub versions configuration. Agents poll every 15 seconds, validate it and
report the applied revision. Saving configuration does not mean the Agent has
already applied it; the page shows pending/applied/error state.

Private and loopback probe targets are disabled by default. Add these fields to
the local Agent TOML/JSON configuration only for the networks it should inspect:

```toml
allow_private_probe_targets = true
allow_loopback_probe_targets = true
```

Link-local, cloud metadata, unspecified and multicast targets remain prohibited.
All resolved addresses are checked, and HTTP DNS answers are pinned. Probe
requests do not include Hub/Agent credentials and do not follow redirects.
Response content matching reads at most 64 KiB. Linux ICMP permission depends on
the OS ping socket policy; unsupported permissions are reported explicitly, and
the Agent is not automatically elevated.

## Visitor-to-node latency

The card's access latency comes from that visitor's browser directly requesting
the configured node HTTPS URL. It is HTTP application round-trip duration,
including the route and endpoint processing, not ICMP or one-way latency.
Agent-to-Hub upload duration remains in the Collection Quality tab.

Each monitored host needs its own reachable HTTPS endpoint. NAT-only nodes
without such an endpoint display unconfigured/unavailable. Routing the measurement
through the Hub does not measure browser-to-node latency.

The Agent provides an optional empty response endpoint. It is disabled by
default. To enable it behind an existing HTTPS reverse proxy, configure:

```toml
latency_bind = "127.0.0.1:18091"
dashboard_origin = "https://monitor.example.com"
```

The bind address must be loopback. The dashboard origin must be an exact HTTPS
origin without credentials, path or trailing slash. Restart that Agent after
editing its local configuration. No installer opens a public measurement port.

Example Caddy route on the monitored host:

```caddyfile
node01.example.com {
    handle /pinglake/latency {
        reverse_proxy 127.0.0.1:18091
    }
}
```

This is an example for an authorized deployment, not a command that was applied
to production. Preserve any existing site's routes when adding the endpoint.
Configure its node URL as `https://node01.example.com/pinglake/latency`.
Disable any reverse-proxy/CDN caching on that path and retain Cache-Control.

The endpoint returns only an empty 204 response and no credentials/metrics.
It permits the configured dashboard Origin and same-origin requests without an
Origin header, rejects other origins and caps requests at 100 per second.
Endpoint startup failure is shown as a capability error; host reporting continues.

The dashboard CSP allows only configured measurement origins. After adding a
new origin, reload the page to obtain the updated CSP. Browsers may refuse local
network access, invalid TLS or missing CORS; the UI reports unavailable rather
than substituting Hub latency. Visitor measurements remain browser-session data
and are not accepted as global node health or SLA observations.

## Quality, buffering and history

Collection and upload run independently. The in-memory upload queue holds 64
reports, drops oldest queued reports on overflow and expires samples older than
300 seconds. Drop counts are reported; this is not persistent offline archival.
Detailed metrics are trimmed if needed to keep the report below the 256 KiB
ingestion limit, with a visible report_budget capability warning.

Upload success rate uses the last 100 attempts, not only successful deliveries.
The outcome of an upload appears in a subsequent report. During complete network
failure, the Hub observes missing receipts; local failure counters become visible
after reconnection. Collection/send durations and sample age are separate.

Raw data stays for seven days. Detailed resource history returns at most 240
time-bucket representative snapshots with a 4 MiB response limit; it is not a
peak-preserving aggregate. Select a metric/device to keep responses bounded.
Service/probe history uses independent deduplicated event tables. Collection and
Hub receipt time are returned separately; legacy history time semantics remain.

Probe P50/P95/P99 use successful raw durations with nearest-rank quantiles,
reported sample count and small-sample indicators. Failures are not zero latency.
Success rate is successful checks / actual valid attempts. Coverage uses unique
scheduled slots and versioned intervals; missing/offline/unsupported checks stay
unknown. Observed availability and unknown duration are separate from coverage.

Statistics combine historical configurations only when the current target's
kind/address/port/HTTP expectations are identical. Changing a target definition
does not mix unrelated latency samples. Deleted targets remain in raw history.

Service/probe alarms deduplicate by subject and use the existing sustained-time,
Webhook and SMTP settings. Unknown data does not falsely recover an alarm.
Disabling/removing a failed check currently preserves its active incident record;
this iteration does not add manual acknowledgement or incident cancellation.

## Migration and verification

Upgrade the Hub before Agents. SQLite schema v5 adds monitoring JSON, event
tables, configuration versions, deduplication indexes and alert subjects.
Use SQLite's online backup API or stop the Hub for a consistent backup that
includes committed WAL data. Rolling back to v4 requires its compatible database
backup as well as the old binary.

Local regression commands:

```powershell
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
npm --prefix web test
npm --prefix web run build
.\scripts\e2e-smoke.ps1 -Port 18090 -LeaveRunning
.\scripts\e2e-monitoring.ps1 -RunRoot <run_root-from-smoke>
```

The monitoring smoke script uses only its named project-local test run and
controlled loopback targets. It leaves the test Hub/Agents running for inspection.
Its endpoint checks verify the local HTTP/CORS handler, not a production HTTPS
browser path. Production HTTPS, browser screenshots, and Windows LocalService
permissions require their own environment checks.

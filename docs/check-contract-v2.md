# PingLake active-check contract v2

This document is the single contract source for the protocol extension in the
protocol worktree. Agent, Hub and Web changes must consume these definitions;
they must not add a second DNS or result channel.

## Version and compatibility

`MONITORING_SCHEMA_V1` remains the existing resource, service, ICMP, TCP and
HTTP contract. `MONITORING_SCHEMA_V2` adds DNS observations, process results,
and read-only local socket observations.

The Hub advertises the highest report schema it accepts in
`EnrollResponse.monitoring_schema_max`. A missing field means schema 1. An
Agent must select the minimum of its maximum and the Hub value and persist that
selection for queued replay. A schema 1 Hub must never receive schema 2-only
fields.

The protocol crate provides `NodeMonitoringConfig::to_wire_json(max_schema)` and
`MetricReport::to_wire_json(max_schema)`. Downstream HTTP clients must use these
methods for configuration responses and report uploads. Downgrading is a wire
projection: it removes `process_checks`, `local_port_checks`, DNS probe
definitions and DNS observations. Clearing an array while leaving an unknown
field in the JSON object is not a downgrade.

Schema 1 downgrade keeps service checks and ICMP/TCP/HTTP probes. A v2-only
probe is omitted. v2-only probe statuses are mapped conservatively to
`unsupported` or `failure`; they must never become a healthy result. Queued
reports retain their original v2 identity and content. On replay to a schema 1
Hub, only the serialized wire projection changes.

Configuration structures are strict: unknown fields and missing required
identity/target fields are rejected. Existing optional fields such as
`enabled`, `interval_secs` and `timeout_ms` retain their v1 defaults. Newly
added result identity fields are `Option` only to read old stored JSON; a v2
wire result must contain them.

## Canonical DNS channel

DNS is represented only as a `ProbeTarget` with `kind: "dns"` and the optional
typed `dns` object. There is no `dns_checks` configuration array and no
`DnsResult` array in `MonitoringData`.

```json
{
  "id": "target-uuid",
  "name": "public-a",
  "kind": "dns",
  "target": "example.test",
  "port": null,
  "enabled": true,
  "interval_secs": 30,
  "timeout_ms": 5000,
  "expected_status": null,
  "response_contains": null,
  "dns": {
    "record_type": "A",
    "expected_value": "192.0.2.1"
  }
}
```

`record_type` is an enum with the wire values `A` and `AAAA`. The Agent only
performs a DNS lookup and does not connect to an address returned by the
lookup. `expected_value` is optional. When present, it is compared with exact
normalized IP equality; substring, regex, suffix and case-insensitive text
matching are not allowed. With no expected value, a valid `NOERROR` response
with at least one answer is a healthy observation.

The result uses the existing `ProbeResult` channel:

```json
{
  "sample_id": "sample-uuid",
  "target_id": "target-uuid",
  "config_revision": 4,
  "kind": "dns",
  "scheduled_at": "2026-10-01T00:00:00Z",
  "completed_at": "2026-10-01T00:00:00.012Z",
  "status": "success",
  "healthy": true,
  "latency_ms": 1.2,
  "http_status": null,
  "error": null,
  "dns": {
    "record_type": "A",
    "rcode": 0,
    "answers": ["192.0.2.1"]
  }
}
```

`DnsObservation.rcode` is the numeric DNS wire RCODE. `0` is `NOERROR`, `1`
is `FORMERR`, `2` is `SERVFAIL`, `3` is `NXDOMAIN`, `4` is `NOTIMP`, and `5`
is `REFUSED`; other values remain representable. `null` means the resolver did
not receive a DNS response. Answers are empty on an error response. The Hub
must preserve the RCODE and must not convert it to a successful zero or empty
answer.

## Process checks

`ProcessCheck` is a read-only exact process-name check. `process_name` is not a
shell command, glob, regular expression or command-line fragment. The Agent
reports the number of visible matching processes and evaluates the configured
expectation.

Every v2 `ProcessResult` contains `sample_id`, `config_revision`,
`scheduled_at`, and `completed_at`. `status` describes whether the Agent could
collect the observation. `healthy` is `true` or `false` only when collection
succeeded; it is `null` for permission denied, unsupported, stale or otherwise
unavailable data. `count: null` means no count was collected. Zero is a real
collected count and is never used as a substitute for unavailable data.

## Local port checks

`LocalPortCheck` is a read-only inspection of the local socket table. It does
not open a connection and its `latency_ms` remains `null` for v2 results.
`protocol` is explicitly `tcp` or `udp`; `address_family` is `any`, `ipv4`, or
`ipv6`; and `address_scope` is `any_local`, `loopback`, or an exact IP address.
The Hub/Agent validation layer must require all three fields for a v2 wire
configuration and require `port` to be 1..65535.

`LocalPortResult` carries the same four execution identity fields as
`ProcessResult`, plus the selected scope, family, protocol, port and the
observed local addresses. `status` is collection state and `healthy` is true
when a matching socket was observed, false when the query succeeded and no
matching socket was observed, and null when the table could not be collected.

The result must preserve IPv4/IPv6 addresses and TCP/UDP semantics. An empty
`observed_addresses` array with `permission_denied` or `unsupported` is not a
negative observation and must not open an alert as if the port were closed.

## Identity, retries and conflict handling

The report identity is `(node_id, monitoring.session_id,
monitoring.sample_sequence)`. Each process or local-port execution also has a
`sample_id`; DNS uses the existing `ProbeResult.sample_id`. The Hub stores the
first accepted payload for an identity and treats an identical retry as a
successful idempotent replay.

For a same identity with different content, the Hub must return a conflict and
record the conflict. It must not silently replace the first payload or merge
the two observations. The comparison uses SHA-256 of
`MetricReport::canonical_content_bytes()` (or byte equality before hashing).
That canonical projection excludes schema/session/sequence envelope fields and
the complete `MonitoringData.agent` transport object, including
`sample_age_ms`, upload counters, queue length, last error and send duration.
Changing transport metadata therefore cannot create a content conflict;
changing a collected metric or check result does.

## Limits and validation

Every check definition contains an interval and timeout. The normal limits are
10..86,400 seconds for the interval, 1..10,000 milliseconds for timeout, and a
timeout strictly shorter than the interval. IDs, names, hostnames, process
names, error text, DNS answer counts and report size remain bounded by the Hub
validation contract. A v2 Agent must emit at most 32 definitions of each
configured check class and at most 64 DNS answers per result.

`status` and `healthy` must be interpreted as separate fields throughout Hub,
statistics, alerts and Web. `unsupported`, `permission_denied`, `policy_denied`,
`stale` and missing observations are unknown states. They never become zero
latency, zero process count, closed port, empty healthy DNS or an alert recovery.

## Integration order

1. Merge this protocol commit and fixtures first. No downstream window may add
   another DNS/result shape.
2. Update Hub validation and storage to accept v1 and negotiated v2, enforce
   the limits above, persist DNS RCODE and process/local-port identities, and
   implement same-identity hash conflicts.
3. Update Agent scheduling and collectors. Use the negotiated schema for both
   new uploads and queued replay; use the protocol wire projection when the Hub
   only accepts v1.
4. Update Web types and views from these exact fields. Hide v2 controls when
   the node capability is absent and show collection status separately from
   target health.
5. Run compatibility fixtures with old/new Hub and Agent combinations, then
   perform Linux/Windows permission tests and real deployment rollback tests.

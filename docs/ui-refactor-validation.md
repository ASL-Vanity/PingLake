# UI refactor validation

Date: 2026-10-01 (Asia/Shanghai)

## Implemented

- The monitoring configuration editor now exposes DNS, process and local-port
  checks as separate typed sections. Process expectations, DNS record type and
  expected answer, local bind address, interval and timeout are editable and
  normalized before save.
- Node details render DNS answers, process instance counts and local listening
  port results independently. `policy_denied`, `permission_denied`,
  `timeout`, `unavailable` and stale results retain separate status labels;
  v1 nodes with absent extension arrays continue to render normally.
- History queries accept the v2 event sections `dns`, `processes` and
  `local_ports`, with event timestamps and metric selectors kept separate from
  resource history.
- Monitoring configuration rows collapse to one or two columns on narrow
  screens while preserving native form controls and keyboard focus order.

- Seven navigation destinations: overview, hosts, services, probes, alerts,
  groups and settings. Desktop sidebar and mobile bottom navigation retain
  direct access to each destination.
- Overview shows compact fleet counts, abnormal hosts and the five newest
  alerts. Resource averages no longer dominate the first screen.
- Host cards and a dense table share search, state/group filters and six sort
  choices. View/sort preferences persist; resource labels remain visible.
- Group management is a separate view. Populated-group deletion preserves
  nodes and history; failed assignments remain visible.
- Fleet service/probe views query node configurations with four workers,
  cancel on navigation and distinguish pending, paused, unknown, offline and
  expired checks. Links open the relevant host detail tab.
- Host details use a single overview/resource/network/service/probe/quality
  tab bar. Configuration is a separate tool action. Duplicate resource
  sections and competing time selectors have been removed.
- One 1/6/24-hour/7-day time range applies to device history and statistics.
  Old requests cannot overwrite new selections. Same-device range changes
  retain the previous graph while loading, with a clear previous-range state;
  different devices do not reuse unrelated data.
- Sparse probe history keeps every real observation. Missing intervals insert
  line breaks instead of discarding the next valid sample.
- Settings render as a normal page, retain theme choices and lock fields during
  submission. Dark controls and surfaces use shared theme variables.
- Hash routing preserves page, host, detail tab, search and group filter;
  browser history navigation restores state. Manual refresh reloads detail
  configuration and trends as well as fleet snapshots.

## Verification

- TypeScript check and Vite production build pass.
- Twenty frontend tests pass: four measurement tests, four host filter/sort
  tests and twelve DOM/history interaction tests.
- DOM tests cover navigation/history, card/table changes, offline display,
  assignment errors, populated-group cancellation/deletion, shared time ranges,
  stale response rejection, old-range retention, new-device request errors,
  sparse event observations, expired/pending checks, configuration refresh
  and submission locking.
- The existing debug Hub at http://127.0.0.1:18090 serves the rebuilt index,
  scripts, styles and original brand asset. No production rollout was performed.
- Browser visual QA remains incomplete: the browser tool could not verify saved
  permissions for the local site. DOM fixtures do not verify pixels, responsive
  geometry, chart framing or real browser focus behavior.
- A separate Vite process launch was rejected with `blocked by policy`; the
  existing local test Hub provides the updated build without launching another
  service.

## Files and commands

```powershell
npm --prefix web test
npm --prefix web run typecheck
npm --prefix web run build
```

UI fixture bundles are generated under ignored web/.tmp. Test-only jsdom is a
development dependency; it does not enter the production dashboard bundle.
Current running test credentials remain in the existing ignored test run's
control.json, and are not duplicated in this document.

## Visual refinement

A subsequent visual pass adds a graphite navigation surface, neutral light/dark
workspace tokens, readable typography and tabular numerals, restrained shadows
on individual host cards, consistent control shapes, and improved summary and
table hierarchy. The login is now an unframed centered form using the existing
brand mark; the decorative READY panel was removed.

All twenty existing tests and production build pass after this pass. The local
Hub serves the latest files. Browser permission verification still prevents
desktop/mobile screenshots, so visual geometry is not marked verified.

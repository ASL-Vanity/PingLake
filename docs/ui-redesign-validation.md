# UI Redesign Validation

Date: 2026-10-01 (Asia/Shanghai)

The prior PingLake dashboard styling was removed from the load path. The new
console uses `web/src/console.css`, the rebuilt `OverviewDashboard`, the new
host workspace, and the rebuilt inspection detail view. Old `styles.css`,
`design.css`, the former summary card component and the drawer stylesheet are
no longer loaded.

The new visual system is intentionally operational and restrained:

- Bright canvas and white work surfaces with graphite navigation.
- One shared ink/muted/status token set across light, dark, midnight and circuit
  themes.
- Fleet summary band, real history trend selection, host status matrix and
  alert activity on the first screen.
- Host cards/table use readable names, explicit CPU/memory/disk labels, network
  direction labels, visitor latency and last-sample state.
- Detail view uses device inspection hierarchy, a single tab row, shared history
  range and real resource/network/service/probe tables.
- No generated demo telemetry, decorative gradients, hero panels or nested card
  shells were introduced.

Validation completed:

- `npm --prefix web run typecheck`
- `npm --prefix web test`: 20 passed
- `npm --prefix web run build`
- Local test Hub rebuilt and served the new `index-Bu3q6SmJ.js` and
  `index-DXSXuqoi.css` assets at `http://127.0.0.1:18090/`.
- Browser assets returned HTTP 200, including the lazy detail JavaScript/CSS and
  existing PingLake brand mark.

The in-app browser still fails its saved-site permission verification after the
local origin was explicitly added as browse/always-allow. Its page is reachable
through HTTP and DOM fixture tests pass, but desktop/mobile screenshots remain
unverified until Codex is restarted after other active conversations finish.

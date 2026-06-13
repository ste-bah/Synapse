# Synapse Command Center — Visual Design Language

> **Single source of truth.** Every dashboard view (#912, #916–#922, #870) consumes these tokens.
> No component may hardcode a hex, an off-scale px, or a font name — only `var(--token)` (from
> [`tokens.css`](./tokens.css)) or the Tailwind utilities re-exported by [`theme.css`](./theme.css).
> This is the concrete theme the UX charter #945 references. Issue: #946.

## Aesthetic direction — calm mission-control for an AI agent fleet
Reference points: **Linear** (dark canvas, single accent, surface-lift over shadows, hairline
borders), **Vercel/Geist** (systematic, monospace-influenced), **Palantir Foundry** (data-first,
serious, dense). Dark-first. **One** chromatic accent, deployed scarcely (brand mark, focus ring,
one primary action per section). Hierarchy comes from **surface lift + hairlines + weight +
spacing**, not shadows or rainbow color. Quiet by default; color appears only to carry meaning
(status). Reads as software-craft, not a toy.

## Files
| File | Role |
|---|---|
| `tokens.css` | OKLCH custom properties — canvas/surface ladder, borders, text ramp, accent, semantic status, type/space/radius/motion/layout. Dark = `:root`; light = `:root[data-theme="light"]`; compact = `:root[data-density="compact"]`. |
| `theme.css` | Tailwind v4 `@theme inline` re-export → utilities (`bg-canvas`, `text-secondary`, `text-success`, `border-strong`, `p-2`, `rounded-md`, `font-mono`). |
| `DESIGN.md` | This spec — the agent-readable contract. |

Generated/illustrative art (logo, empty-state illustrations) is produced with the local
image-generation skill and vendored under `dashboard/` — never a CDN.

## Color — OKLCH, dark-first
Authored in OKLCH for perceptually-uniform ramps; semantic tokens name **intent, not color**.
Hierarchy = a 4-step surface lift (`--canvas` → `--surface-3`) plus three hairline weights. Text is
a 4-step ramp. There is exactly one brand accent (lavender-blue `--accent`), used scarcely. See
`tokens.css` for the resolved values; final L-steps were locked by the contrast audit below.

### Single accent
`--accent` was darkened from the L=0.62 starting point to **L=0.54** so white label text on an
accent-filled primary button clears WCAG AA (4.5:1) — see audit. `--focus-ring` (L=0.74) stays
brighter so the focus indicator is loud on every surface. The accent appears once per section, max.

## Attention-state system — the color-blind-safe heart
The 7 fleet states (#916) are **quadruple-encoded: hue + shape + icon + text label**, so they are
unmistakable in grayscale, under glare, and to the ~8% of men with red-green CVD. **Never color
alone** (WCAG 1.4.1). Red is reserved for genuinely urgent only (Astro/NASA rule). Badges use a
tinted background + darker foreground + a solid border (survives Windows forced-colors mode), never
a saturated fill. Every indicator ships an `aria-label` (e.g. `Status: needs input`).

| State | Token | Shape | Icon | Interrupts? |
|---|---|---|---|---|
| Working | `--info` | ● pulsing dot | activity | no (quiet) |
| Idle | `--text-muted` | ○ hollow dot | pause | no (quiet) |
| Ready for review | `--success` | ◧ half-square | inbox/eye | surfaced, not loud |
| Needs input | `--warning` | ◆ diamond | chat-question | **yes** |
| Awaiting approval | `--warning` | ⚑ flag | shield/check | **yes** (risk-gated, #925) |
| Stuck / error | `--danger` | ▲ triangle | alert | **yes (highest)** |
| Done | `--success` | ● filled / ✓ | check | no (quiet) |

Two pairs deliberately share a hue (Ready/Done = `--success`; Needs-input/Awaiting = `--warning`);
**shape + icon + label** disambiguate them with color removed — verified below.

## Typography
- **UI:** Inter (tall x-height, crisp at 11–13px on Windows ClearType). **Data/code/terminal:**
  JetBrains Mono. Both vendored — no Google Fonts CDN.
- `font-variant-numeric: tabular-nums` globally on data containers (aligned columns).
- Dense modular scale, base **14px / ratio ~1.2**; hierarchy comes mostly from weight + color.

| Token | px | weight | use |
|---|---|---|---|
| `text-label` | 11 | 500, +0.04em, uppercase | metadata labels, table headers |
| `text-xs` | 12 | 400 | secondary metadata |
| `text-sm` | 13 | 400 | table cells, terminal (mono, lh 1.5) |
| `text-base` | 14 | 400 (lh 1.5) | body default |
| `text-md` | 16 | 500 | card titles |
| `text-lg` | 18 | 600 | section headers |
| `text-xl` | 20 | 600 | page subtitle |
| `text-2xl` | 24 | 600, -0.01em | page title |
| `text-display` | 30 | 600, -0.02em | rare hero |
| `text-metric` | 28–36 | 600, tabular | big stat-card numbers (≥1.6× their label) |

## Spacing — 4px base unit
`0:0  0.5:2  1:4  2:8  3:12  4:16  6:24  8:32  12:48  16:64  24:96`. 8px is the dominant rhythm;
4px sub-grid for tight intra-component gaps. Consistent throughout — mixing 16 and 24 arbitrarily
breaks the precision the aesthetic depends on.

## Radius, elevation, motion
- Radius: `sm:4 md:6 lg:8 xl:12 pill:9999`. Moderate — not sharp, not consumer-soft.
- Elevation (dark): hierarchy = **surface lift + hairline border**; one soft `--shadow-overlay`
  token for popovers/overlays only. No glows, no gradients.
- Motion: `--motion-micro:120ms` / `--motion-standard:200ms`, ease-out; status pulse ≤1s.
  **`prefers-reduced-motion` disables the pulse and transitions** (color/icon change stays).

## Layout grid
Sidebar `264px`; content max `1440px` (ops-dense tables may go full-bleed); gutter `24px`; card gap
`16px`; 12-col. **Density toggle** (comfortable/compact) via `[data-density="compact"]`: row height
`44/32`, card padding `16/12`, persisted in client state (#912).

## Component anatomy (built once in the shared package, #912)
`StatCard` (label 11px-muted, metric 28–36 tabular, delta badge), `StatusBadge`/`StatusDot` (the
quadruple-encoding above), `ToolCallCard` (lifecycle chip + one-line summary + disclosure,
charter #945 law 2), `TranscriptTurn`, `FleetRow`, `DataTable` (sticky header, subtle dividers not
zebra, density-aware), `Section`, `EmptyState`. Each documents the tokens it consumes.

---

## Contrast audit — WCAG 2.1 (recorded evidence)

**Method (no re-implementation):** `tokens.css` was loaded in real Chromium; each token was resolved
through the browser's own OKLCH engine by rasterizing it to a 1×1 canvas and reading the sRGB bytes
via `getImageData`. WCAG 2.1 relative-luminance + contrast ratio were then computed for every
text-on-surface and status pair, in **both** themes. Thresholds: **4.5:1** normal text, **3:1**
large text / UI-component & graphical objects (status dots, focus ring, interactive borders).

**Result: every text-on-surface and status pair passes AA in both themes** (39 pairs each; 0
failures). `text-disabled` is intentionally AA-exempt (disabled state is never load-bearing). The
two decorative hairlines (`--border-subtle`, `--border`) are exempt non-text dividers — structure is
carried by surface lift, not the line — and measure ~1.6:1 by design.

### Dark (default) — contrast ratios
Text ramp on `[canvas, surface-1, surface-2, surface-3]`:

| Text token | canvas | surface-1 | surface-2 | surface-3 | min | AA |
|---|---|---|---|---|---|---|
| `text-primary` | 17.83 | 16.60 | 15.10 | 13.84 | 13.84 | ✅ |
| `text-secondary` | 10.37 | 9.65 | 8.78 | 8.05 | 8.05 | ✅ |
| `text-muted` | 6.25 | 5.82 | 5.29 | 4.85 | 4.85 | ✅ |
| `text-disabled` | 3.50 | 3.26 | 2.97 | 2.72 | — | exempt |

Status dot on `surface-1` (≥3:1): success **8.95**, warning **10.23**, danger **5.74**, info **8.01** ✅
Badge fg on badge bg (≥4.5): success **10.34**, warning **10.05**, danger **8.30**, info **9.90** ✅
Primary button (`accent-fg`/`accent`): **4.92** ✅ · `border-strong`/`surface-1`: **3.57** ✅ · focus ring/canvas: **8.24** ✅

### Light — contrast ratios

| Text token | canvas | surface-1 | surface-2 | surface-3 | min | AA |
|---|---|---|---|---|---|---|
| `text-primary` | 16.90 | 16.10 | 15.00 | 14.08 | 14.08 | ✅ |
| `text-secondary` | 9.73 | 9.26 | 8.63 | 8.10 | 8.10 | ✅ |
| `text-muted` | 5.83 | 5.56 | 5.18 | 4.86 | 4.86 | ✅ |
| `text-disabled` | 3.30 | 3.14 | 2.93 | 2.75 | — | exempt |

Status dot on `surface-1` (≥3:1): success **4.06**, warning **3.46**, danger **5.00**, info **4.34** ✅
Badge fg on badge bg (≥4.5): success **6.74**, warning **7.27**, danger **6.68**, info **6.57** ✅
Primary button: **4.88** ✅ · `border-strong`/`surface-1`: **3.97** ✅ · focus ring/canvas: **4.85** ✅

### Grayscale & icon-only tests
The 7-state swatch was rendered in real Chromium and screenshotted with `filter: grayscale(1)`.
With **all color removed**, every state stayed distinguishable by shape + icon + text — including
the two same-hue pairs: Ready-for-review (`◧` half-square) vs Done (`●`/`✓`), and Needs-input
(`◆`/`?`) vs Awaiting-approval (`⚑`/`⛉`). This confirms the quadruple-encoding (WCAG 1.4.1):
color is never the sole carrier of state.

> Re-run the audit after any token change: load `tokens.css` in a browser and compute WCAG ratios
> for the pairs above against 4.5 (text/badge-text) and 3.0 (status/focus/interactive-border),
> for `:root` and `:root[data-theme="light"]`.

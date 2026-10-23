# MIRASET Wallet — Visual Style Design

> Companion to `docs/WALLET_DESIGN.md`. Scope: the look and feel of the desktop wallet.
> Targets the current stack — **Next.js 16 + React 19 + Tailwind v4** — and the existing
> `wallet/src/app/globals.css` / `wallet/src/app/page.tsx` files. All file references are
> relative to the repo root.

## 1. Goals

1. **One source of truth** for color, spacing, type, radius, motion, and elevation —
   expressed as CSS custom properties so theming (and runtime tweaks) stay cheap.
2. **Stop repeating Tailwind utilities** in `page.tsx` (e.g. `rounded-xl border
   border-[#24262c] bg-[#14161a] p-6` appears verbatim at least seven times).
3. **Semantic naming**: every visual value should answer *"what is this for?"* before
   *"what does it look like?"*. `bg-panel` is better than `bg-[#14161a]`.
4. **Theme-ready**: light, dark, and a "system" mode should be first-class, not a
   follow-up. The current app is dark-only by hex value.
5. **Accessibility built in**: focus rings, reduced-motion, color contrast, hit targets
   ≥ 40 px.
6. **No JS / no new dependencies** — the entire system is reachable from
   `globals.css` and a small set of Tailwind v4 `@utility` rules.

## 2. Current state — what we're fixing

`wallet/src/app/globals.css:1-29` is 29 lines. It only declares four CSS variables
(`--background`, `--foreground`, `--accent`, `--panel`, `--panel-border`) and one
utility class (`.font-display`). Everything else is hard-coded in JSX:

| Pattern | Repeated as | Count in `page.tsx` |
|---|---|---|
| Card surface | `rounded-xl border border-[#24262c] bg-[#14161a] p-6 shadow-lg` | 7+ |
| Inner surface | `rounded-md border border-[#24262c] bg-[#0f1115] ...` | 15+ |
| Input | `rounded-md border border-[#24262c] bg-[#0f1115] px-3 py-2 text-sm text-zinc-100 placeholder:text-zinc-500` | 6 |
| Primary button | `rounded-md bg-[#f7931a] px-4 py-2 text-sm font-semibold text-black` | 4 |
| Secondary button | `rounded-md border border-[#24262c] px-4 py-2 text-sm text-zinc-200` | 6 |
| Status dot (online) | `h-2 w-2 rounded-full bg-emerald-400` | 3 |
| Status dot (offline) | `h-2 w-2 rounded-full bg-red-400` | 3 |
| Status dot (checking) | `h-2 w-2 rounded-full bg-amber-400` | 3 |
| Status pill | `flex items-center gap-2 rounded-full border border-[#24262c] px-3 py-1` | 3 |
| Section label | `text-xs font-semibold uppercase tracking-[0.2em] text-zinc-400` | 4 |

That gives us the diagnosis: the **design language exists**, it just lives in JSX. The
fix is to lift it into CSS.

## 3. Design principles

1. **Dark-first, but not dark-only.** MIRASET is a chain client — most users keep it open
   in the background. Dark is the default; light and "system" are first-class themes.
2. **Monochrome surface, single accent.** Neutrals carry structure; the Bitcoin-orange
   `#f7931a` is reserved for *primary action*, *selected state*, and *important status*.
   Status colors (success/warning/danger) are independent of the brand accent.
3. **Type does the talking.** Display family (`Bebas Neue`) for headlines; `Ubuntu` for
   body; a monospace stack for addresses, hashes, and hex. The ratio between them
   encodes hierarchy.
4. **No gradients, no drop shadows on text, no glass.** The product is functional; the
   visual language is "control panel", not "marketing site".
5. **8 px base unit.** Spacing, radii, and most component sizes are multiples of `0.5rem`
   (8 px). `0.25rem` is allowed for tight in-component adjustments only.
6. **State is silent until it isn't.** Default state carries no shadow. Elevation,
   borders, and color shifts only appear on interaction or to group.

## 4. Design tokens

All tokens are CSS custom properties on `:root`, exposed as Tailwind v4 theme variables
via `@theme inline` in `globals.css`. Naming uses a `--{category}-{role}-{variant}`
pattern.

### 4.1 Color tokens

#### Surface (backgrounds)

| Token | Dark | Light | Role |
|---|---|---|---|
| `--surface-base` | `#0b0d10` | `#f8f8f7` | App background |
| `--surface-raised` | `#14161a` | `#ffffff` | Card / panel |
| `--surface-sunken` | `#0f1115` | `#f1f1ef` | Input, inset list |
| `--surface-overlay` | `rgba(11,13,16,0.72)` | `rgba(255,255,255,0.72)` | Modal scrim |
| `--surface-hover` | `rgba(255,255,255,0.04)` | `rgba(0,0,0,0.04)` | Row hover |
| `--surface-selected` | `rgba(247,147,26,0.10)` | `rgba(247,147,26,0.10)` | Selected row / active nav |

#### Border

| Token | Dark | Light | Role |
|---|---|---|---|
| `--border-subtle` | `#1d1f24` | `#ececea` | Hairline divider |
| `--border-default` | `#24262c` | `#e2e2df` | Card / input edge |
| `--border-strong` | `#3a3d44` | `#cdcdca` | Hover, focus surround |
| `--border-accent` | `#f7931a` | `#f7931a` | Selected / brand |

#### Text

| Token | Dark | Light | Role |
|---|---|---|---|
| `--text-primary` | `#f5f5f4` | `#18181b` | Headings, balances |
| `--text-secondary` | `#d4d4d8` | `#3f3f46` | Body, labels |
| `--text-muted` | `#a1a1aa` | `#71717a` | Metadata, captions |
| `--text-disabled` | `#52525b` | `#a1a1aa` | Disabled controls |
| `--text-inverse` | `#0b0d10` | `#ffffff` | On-accent text |
| `--text-accent` | `#f7931a` | `#b86f0c` | Links, focus, badges |

#### Brand (single accent)

| Token | Value | Role |
|---|---|---|
| `--brand-500` | `#f7931a` | Default accent |
| `--brand-600` | `#e0850f` | Hover |
| `--brand-700` | `#c67408` | Active / pressed |
| `--brand-soft` | `rgba(247,147,26,0.12)` | Selected surface, badge bg |
| `--brand-ring` | `rgba(247,147,26,0.45)` | Focus ring |

#### Status (semantic, not brand)

| Token | Dark | Light |
|---|---|---|
| `--success-fg` / `--success-soft` | `#34d399` / `rgba(52,211,153,0.12)` | Online, completed |
| `--warning-fg` / `--warning-soft` | `#fbbf24` / `rgba(251,191,36,0.12)` | Checking, pending |
| `--danger-fg`  / `--danger-soft`  | `#f87171` / `rgba(248,113,113,0.12)` | Offline, error |
| `--info-fg`    / `--info-soft`    | `#60a5fa` / `rgba(96,165,250,0.12)` | Neutral info |

These map to Tailwind's `emerald-400`, `amber-400`, `red-400`, `blue-400` shades used
in `page.tsx:544-583` and `page.tsx:1013-1064` — keep them aligned so status dots and
status banners never disagree.

### 4.2 Typography

| Token | Value | Use |
|---|---|---|
| `--font-display` | `"Bebas Neue", system-ui, sans-serif` | Section titles, brand |
| `--font-sans` | `"Ubuntu", system-ui, sans-serif` | Body, buttons, inputs |
| `--font-mono` | `"JetBrains Mono", ui-monospace, "SFMono-Regular", Menlo, monospace` | Addresses, hashes, hex |

Currently `page.tsx` uses `font-display` (Bebas) for `<h1>`, `<h2>` and `font-sans` for
everything else. Address/hex strings are rendered in the body font and clipped — they
should be `font-mono` with `tabular-nums` and an explicit letter-spacing reset.

Type scale (rem, 16 px base):

| Token | rem | px | Use |
|---|---|---|---|
| `--text-2xs` | 0.6875 | 11 | Eyebrow / overline |
| `--text-xs` | 0.75 | 12 | Table cells, captions |
| `--text-sm` | 0.875 | 14 | Form labels, buttons |
| `--text-base` | 1 | 16 | Body |
| `--text-lg` | 1.125 | 18 | Subheadings |
| `--text-xl` | 1.5 | 24 | Card titles (`font-display`) |
| `--text-2xl` | 2 | 32 | Section titles (`font-display`) |
| `--text-3xl` | 3 | 48 | Page H1 (`font-display`) |
| `--text-display` | 4 | 64 | Balance hero (optional, future) |

Line heights: `1.2` for display, `1.45` for body, `1.1` for mono numerics. Letter
spacing: `0.04em` on display, `0.2em` on overlines (already used in `page.tsx:538`,
`page.tsx:605`, `page.tsx:733`).

### 4.3 Spacing & sizing

| Token | Value | Use |
|---|---|---|
| `--space-0` … `--space-12` | 0, 2, 4, 8, 12, 16, 20, 24, 32, 40, 48, 64, 96 px | 4 px grid |
| `--radius-sm` | 4 px | Pills, small badges |
| `--radius-md` | 6 px | Inputs, inline buttons |
| `--radius-lg` | 12 px | Cards |
| `--radius-xl` | 16 px | Modals, hero cards |
| `--radius-full` | 9999 px | Avatars, status dots |
| `--control-sm` | 28 px | Inline / dense |
| `--control-md` | 36 px | Default form / button |
| `--control-lg` | 44 px | Primary CTA |
| `--hit-target` | 40 px | Minimum tap target (a11y) |

### 4.4 Elevation & motion

| Token | Value | Use |
|---|---|---|
| `--shadow-sm` | `0 1px 0 0 var(--border-subtle)` | Card hairline |
| `--shadow-md` | `0 4px 12px -2px rgba(0,0,0,0.35)` | Popover |
| `--shadow-lg` | `0 12px 32px -8px rgba(0,0,0,0.45)` | Modal |
| `--ring-focus` | `0 0 0 2px var(--surface-base), 0 0 0 4px var(--brand-ring)` | Standard focus |
| `--ease-out` | `cubic-bezier(0.2, 0.8, 0.2, 1)` | Default |
| `--ease-in` | `cubic-bezier(0.6, 0, 0.8, 0.2)` | Exit |
| `--dur-fast` | `120ms` | Color / opacity |
| `--dur-base` | `200ms` | Transform |
| `--dur-slow` | `320ms` | Layout |

All motion tokens honor `@media (prefers-reduced-motion: reduce)` by collapsing
`--dur-*` to `0ms`.

## 5. Semantic color mapping

Tokens map to intent so JSX reads in plain English. Examples:

```html
<button class="bg-brand-500 hover:bg-brand-600 text-text-inverse">Send</button>
<input  class="bg-surface-sunken border border-border-default text-text-primary" />
<div    class="border border-border-subtle bg-surface-raised shadow-shadow-sm">…</div>
<span  class="bg-success-soft text-success-fg">Online</span>
```

The "address display" pattern used in `page.tsx:843` becomes:

```html
<div class="font-mono text-xs text-text-secondary break-all">{address}</div>
```

…instead of the current `rounded-md border border-dashed border-[#24262c] bg-[#0f1115]
px-3 py-2 text-xs text-zinc-300`.

## 6. Component styles

Each component is a Tailwind v4 `@utility` or `@layer components` rule in
`globals.css`. The list below is the **complete** vocabulary the wallet needs — every
inline class in `page.tsx` should map to one of these.

### 6.1 Surfaces

| Component | Class | Spec |
|---|---|---|
| Card | `.card` | `bg-surface-raised border border-border-default rounded-radius-lg p-6 shadow-shadow-sm` |
| Card inset | `.card-inset` | Same as card but `bg-surface-sunken` and no shadow |
| Section | `.section` | Vertical stack with `--space-6` gap |
| Panel header | `.panel-header` | `flex items-center justify-between pb-4 mb-4 border-b border-border-subtle` |
| Banner | `.banner` | `rounded-radius-md px-4 py-2 text-sm` + variant modifier (`.banner-danger`, `.banner-warning`, `.banner-info`) |

Replaces the seven `rounded-xl border border-[#24262c] bg-[#14161a] p-6` instances and
the three inline `border-red-500/30 bg-red-500/10` / `bg-amber-500/10` banners
(`page.tsx:591-600`).

### 6.2 Buttons

Three sizes (sm / md / lg), four variants (primary / secondary / ghost / danger).

```css
.btn              { @apply inline-flex items-center justify-center gap-2
                     rounded-radius-md font-semibold text-sm
                     transition-colors duration-(--dur-fast) ease-(--ease-out)
                     focus-visible:outline-none focus-visible:ring-(--ring-focus)
                     disabled:opacity-50 disabled:cursor-not-allowed; }
.btn-md           { @apply h-(--control-md) px-4; }
.btn-sm           { @apply h-(--control-sm) px-3 text-xs; }
.btn-lg           { @apply h-(--control-lg) px-5 text-base; }
.btn-primary      { @apply bg-brand-500 text-text-inverse hover:bg-brand-600 active:bg-brand-700; }
.btn-secondary    { @apply bg-surface-raised text-text-primary border border-border-default hover:border-border-strong; }
.btn-ghost        { @apply bg-transparent text-text-secondary hover:bg-surface-hover; }
.btn-danger       { @apply bg-danger-soft text-danger-fg hover:bg-danger-fg hover:text-text-inverse; }
```

The current `rounded-md bg-[#f7931a] px-4 py-2 text-sm font-semibold text-black` button
(`page.tsx:617`, `page.tsx:744`, `page.tsx:830`, `page.tsx:986`) becomes simply
`btn btn-md btn-primary`. The "Refresh" / "Copy Address" / "Export Secret" / "Import"
buttons (`page.tsx:622`, `page.tsx:793`, `page.tsx:799`, `page.tsx:770`) become
`btn btn-md btn-secondary`.

### 6.3 Form controls

```css
.label          { @apply text-xs font-semibold uppercase tracking-[0.2em] text-text-muted; }
.input          { @apply w-full h-(--control-md) rounded-radius-md
                  bg-surface-sunken border border-border-default
                  px-3 text-sm text-text-primary
                  placeholder:text-text-muted
                  focus:outline-none focus:border-border-strong focus:ring-(--ring-focus)
                  disabled:opacity-50; }
.input-error    { @apply border-danger-fg focus:border-danger-fg focus:ring-(--danger-soft); }
.field          { @apply flex flex-col gap-1.5; }
.field-row      { @apply flex gap-2 items-end; }
.help           { @apply text-xs text-text-muted; }
```

Replaces the six inputs at `page.tsx:608-613`, `738-742`, `759-768`, `816-828`, `953-957`,
`965-969`, `977-981`.

### 6.4 Status indicators

The wallet already has three states (online / checking / offline) × three services
(RPC / worker / Ollama) × two locations (header pill + status detail row). A
single component covers all 18 cells:

```css
.status         { @apply inline-flex items-center gap-2 text-xs; }
.status-dot     { @apply h-2 w-2 rounded-full shrink-0; }
.status-pill    { @apply status rounded-full border border-border-default
                  px-3 py-1 text-text-secondary; }
.status-row     { @apply status text-text-secondary; }
.status-online  { @apply bg-success-fg; }
.status-checking{ @apply bg-warning-fg animate-pulse; }
.status-offline { @apply bg-danger-fg; }
```

`page.tsx:541-588` and the detail block at `page.tsx:630-685` collapse to a small
`<StatusPill service="rpc" />` and a `<StatusRow />` pair, with the connection
snapshot driving the state. The two binary indicators at `page.tsx:1013-1019` (workers)
and `page.tsx:1054-1065` (jobs) become `.status .status-online` /
`.status .status-checking` / `.status .status-offline` too.

A short note on `animate-pulse`: it is acceptable because the polling window is 10 s
and the dot is < 8 px. Wrap the rule in `@media (prefers-reduced-motion: no-preference)`
to disable it for sensitive users.

### 6.5 Lists & tables

```css
.list           { @apply flex flex-col gap-2; }
.list-item      { @apply flex items-center justify-between gap-3
                  rounded-radius-md border border-border-default bg-surface-sunken
                  px-3 py-2.5 text-sm; }
.list-item-selected { @apply border-border-accent bg-surface-selected; }

.table          { @apply w-full text-xs text-text-secondary; }
.table thead th { @apply pb-2 pr-3 text-left text-text-muted font-medium; }
.table tbody tr { @apply border-t border-border-subtle; }
.table tbody td { @apply py-1.5 pr-3 tabular-nums; }
```

The transaction-history table at `page.tsx:865-937` becomes a `<table class="table">`,
the worker/job lists at `page.tsx:1007-1029` and `page.tsx:1049-1075` become a
`.list` of `.list-item` rows, and the accounts list at `page.tsx:697-714` becomes the
same with the `list-item-selected` variant for the active account.

### 6.6 Section overline

The four identical "RPC URL" / "New account name" / "Import account" / "Max Tokens"
labels (`page.tsx:605`, `page.tsx:733`, `page.tsx:753`, `page.tsx:949`,
`page.tsx:961`, `page.tsx:973`) collapse to `<label class="label">`.

### 6.7 Address / hash display

```css
.mono           { @apply font-mono text-xs text-text-secondary break-all; }
.mono-truncate  { @apply mono truncate max-w-[40ch]; }
```

Replace `text-xs text-zinc-300` on the address string at `page.tsx:843` and the
sliced `<span className="text-zinc-500 truncate max-w-[120px]">` on
`page.tsx:1023-1024` / `page.tsx:1069-1070`.

## 7. Layout & composition

### 7.1 Container

`page.tsx:531` already uses `mx-auto flex max-w-6xl flex-col gap-6 px-6 py-10`. Keep
it; just expose it as `.page` so we have one place to change it.

```css
.page           { @apply mx-auto flex max-w-6xl flex-col gap-6 px-6 py-10; }
@media (max-width: 640px) {
  .page         { @apply px-4 py-6 gap-4; }
}
```

### 7.2 Grids

Two patterns repeat:

- The accounts / selected-account split: `grid gap-6 lg:grid-cols-[1.1fr_1fr]`
  (`page.tsx:691`).
- The submit-job / workers-jobs split: `grid gap-6 lg:grid-cols-2`
  (`page.tsx:943`).

```css
.split-cards    { @apply grid gap-6 lg:grid-cols-[1.1fr_1fr]; }
.split-equal    { @apply grid gap-6 lg:grid-cols-2; }
```

### 7.3 Inline stacks

Status pills in the header are a horizontal flex that wraps on small screens. Make it
explicit:

```css
.cluster        { @apply flex flex-wrap items-center gap-3; }
```

## 8. Interaction states

| State | Visible change |
|---|---|
| Default | `bg-surface-raised`, `border-border-default` |
| Hover | `border-border-strong` (cards) / `bg-brand-600` (primary) / `bg-surface-hover` (rows) |
| Focus-visible | `box-shadow: var(--ring-focus)` (always 4 px orange ring on the brand surface) |
| Active / pressed | `bg-brand-700` (primary) / scale 0.98 transform (≤ 200 ms) |
| Disabled | `opacity: 0.5; cursor: not-allowed;` — no hover or focus change |
| Loading | Replace label with 14 px spinner, keep width to prevent layout shift |
| Error | `border-danger-fg` + `<p class="help text-danger-fg">…</p>` under the field |

Focus-visible is the only focus state we ship. `:focus` (non-visible) is suppressed via
`focus:not(:focus-visible):{outline:none}` so keyboard users always get the ring and
mouse users don't see a stray outline.

## 9. Theming

Three themes — `dark` (default), `light`, `system` — exposed via the `data-theme`
attribute on `<html>`. Implemented as CSS variable overrides:

```css
:root, :root[data-theme="dark"]  { /* dark token values */ }
:root[data-theme="light"]        { /* light token values */ }

@media (prefers-color-scheme: light) {
  :root[data-theme="system"]     { /* light token values */ }
}
```

A small set of components should react automatically when the user changes
`prefers-color-scheme` while `data-theme="system"`. Because the values are
`var(--surface-*)` etc., the only work is making sure no hex literal slips back into
`page.tsx`.

The accent (`--brand-*`) and status colors stay constant across themes so the visual
language doesn't shift when the user toggles.

## 10. Implementation strategy (Tailwind v4)

Tailwind v4 supports the `@theme inline` directive for token import and `@utility` /
`@layer components` for component classes. The full `globals.css` will be roughly:

```css
@import "tailwindcss";

/* 1. Tokens */
:root,
:root[data-theme="dark"] { /* all dark values */ }
:root[data-theme="light"] { /* all light values */ }
@media (prefers-color-scheme: light) {
  :root[data-theme="system"] { /* all light values */ }
}

/* 2. Token exposure to Tailwind */
@theme inline {
  --color-surface-base:    var(--surface-base);
  --color-surface-raised:  var(--surface-raised);
  --color-surface-sunken:  var(--surface-sunken);
  --color-surface-hover:   var(--surface-hover);
  --color-surface-selected:var(--surface-selected);
  --color-border-subtle:   var(--border-subtle);
  --color-border-default:  var(--border-default);
  --color-border-strong:   var(--border-strong);
  --color-border-accent:   var(--border-accent);
  --color-text-primary:    var(--text-primary);
  --color-text-secondary:  var(--text-secondary);
  --color-text-muted:      var(--text-muted);
  --color-text-disabled:   var(--text-disabled);
  --color-text-inverse:    var(--text-inverse);
  --color-text-accent:     var(--text-accent);
  --color-brand-500:       var(--brand-500);
  --color-brand-600:       var(--brand-600);
  --color-brand-700:       var(--brand-700);
  --color-success-fg:      var(--success-fg);
  --color-success-soft:    var(--success-soft);
  --color-warning-fg:      var(--warning-fg);
  --color-warning-soft:    var(--warning-soft);
  --color-danger-fg:       var(--danger-fg);
  --color-danger-soft:     var(--danger-soft);
  --color-info-fg:         var(--info-fg);
  --color-info-soft:       var(--info-soft);

  --font-display: var(--font-display);
  --font-sans:    var(--font-sans);
  --font-mono:    var(--font-mono);

  --radius-sm: var(--radius-sm);
  --radius-md: var(--radius-md);
  --radius-lg: var(--radius-lg);
  --radius-xl: var(--radius-xl);

  --shadow-sm: var(--shadow-sm);
  --shadow-md: var(--shadow-md);
  --shadow-lg: var(--shadow-lg);
}

/* 3. Reset / base */
@layer base {
  body {
    background: var(--surface-base);
    color: var(--text-primary);
    font-family: var(--font-sans);
  }
  :focus-visible { outline: none; box-shadow: var(--ring-focus); border-radius: var(--radius-sm); }
  ::selection { background: var(--brand-soft); color: var(--text-primary); }
}

/* 4. Component utilities */
@layer components {
  .page        { … }
  .card        { … }
  .btn         { … }
  .input       { … }
  .label       { … }
  .status      { … }
  .list        { … }
  .table       { … }
  .mono        { … }
}

/* 5. Reduced motion */
@media (prefers-reduced-motion: reduce) {
  *, *::before, *::after {
    animation-duration: 0.01ms !important;
    transition-duration: 0.01ms !important;
  }
}
```

`font-display` is preserved as a utility (currently defined in `globals.css:25-28`) so
existing JSX keeps working.

## 11. Migration path

The goal is to make `page.tsx` shorter, not longer. Concretely, in this order:

1. **Land the token block** in `globals.css` and the `@theme inline` mapping. No JSX
   changes yet. The hex literals in `page.tsx` still resolve correctly.
2. **Replace the four high-frequency classes** with components: `.card`, `.input`,
   `.btn`, `.label`. Roughly 25 lines of JSX shrink to 5.
3. **Replace the status indicator** triple with `.status` + state modifier. Header
   pills (`page.tsx:541-588`) and the status detail block (`page.tsx:630-685`) collapse
   to a single `<StatusPill />` component.
4. **Add a `mono` font + truncation utility** and apply it to address/hash strings
   (`page.tsx:843`, `1023-1024`, `1069-1070`).
5. **Add the `data-theme` toggle** to the RPC config card as a single `<select>`.
   Default to `"system"`.
6. **Audit** with `rg "border-\[#" " wallet/src` and `rg "bg-\[#" " wallet/src` — the
   counts should drop to zero.

After step 6, `page.tsx` should be under 600 lines (from 1097) with no hard-coded
hex values, no inline grid templates, and no repeated card/input/button strings.

## 12. What is explicitly *not* in this design

- **No iconography system.** The current wallet ships with no icons. If/when icons
  land, they should be a single-stroke 1.5 px set at 16 / 20 / 24 px, with
  `currentColor` inheritance. Until then, status dots and typographic markers carry
  the visual weight.
- **No motion choreography.** No slide-ins, no spring physics. The only motion is
  color/opacity transitions on hover and `animate-pulse` on the `checking` state.
- **No dark/light per-component overrides.** The whole app flips together. Per-card
  inversion is an anti-pattern for a control-panel product.
- **No brand variants.** A future "MIRASET Pro" or partner skin would replace the
  `--brand-*` tokens only; nothing else needs to change.

## 13. Quick reference — before / after

| Before (in `page.tsx`) | After |
|---|---|
| `rounded-xl border border-[#24262c] bg-[#14161a] p-6 shadow-lg` | `card` |
| `rounded-md border border-[#24262c] bg-[#0f1115] px-3 py-2 text-sm text-zinc-100 placeholder:text-zinc-500` | `input` |
| `rounded-md bg-[#f7931a] px-4 py-2 text-sm font-semibold text-black` | `btn btn-md btn-primary` |
| `rounded-md border border-[#24262c] px-4 py-2 text-sm text-zinc-200` | `btn btn-md btn-secondary` |
| `text-xs font-semibold uppercase tracking-[0.2em] text-zinc-400` | `label` |
| `h-2 w-2 rounded-full bg-emerald-400` (and 5 other inline variants) | `status-dot status-online` (and 2 other state classes) |
| `flex items-center gap-2 rounded-full border border-[#24262c] px-3 py-1` | `status-pill` |
| `rounded-md border border-red-500/30 bg-red-500/10 px-4 py-2 text-sm text-red-200` | `banner banner-danger` |
| `mx-auto flex max-w-6xl flex-col gap-6 px-6 py-10` | `page` |
| `grid gap-6 lg:grid-cols-[1.1fr_1fr]` | `split-cards` |
| `font-mono text-xs text-zinc-300` on address text | `mono` |
| `text-zinc-500 truncate max-w-[120px]` on worker/job id | `mono-truncate` |

Adopting this collapses `page.tsx` to a much more readable form: each section becomes
a small list of semantic components instead of a wall of utilities.

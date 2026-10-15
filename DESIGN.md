---
name: TurboPix
description: A straightforward photo and video viewer — fast, simple, and nothing else
colors:
  slate: "oklch(55% 0.08 250deg)"
  slate-deep: "oklch(45% 0.08 250deg)"
  slate-soft: "oklch(65% 0.06 250deg)"
  brass: "oklch(75% 0.12 80deg)"
  danger: "oklch(55% 0.2 25deg)"
  danger-deep: "oklch(48% 0.2 25deg)"
  success: "oklch(65% 0.18 155deg)"
  success-deep: "oklch(58% 0.18 155deg)"
  paper: "oklch(97.5% 0.005 80deg)"
  paper-2: "oklch(96% 0.005 80deg)"
  surface: "oklch(99% 0.002 80deg)"
  surface-raised: "oklch(100% 0 0deg)"
  ink: "oklch(20% 0.01 260deg)"
  ink-2: "oklch(45% 0.01 260deg)"
  ink-3: "oklch(50% 0.01 260deg)"
  slate-ink: "oklch(45% 0.08 250deg)"
  danger-ink: "oklch(45% 0.2 25deg)"
  hairline: "oklch(90% 0.005 260deg / 50%)"
  glass: "oklch(99% 0.002 80deg / 70%)"
  glass-line: "oklch(0% 0 0deg / 6%)"
  veil: "oklch(20% 0.01 260deg / 45%)"
  veil-deep: "oklch(20% 0.01 260deg / 65%)"
  on-slate: "#ffffff"
  on-brass: "oklch(20% 0.01 260deg)"
  night-canvas: "oklch(14% 0.01 260deg)"
  night-surface: "oklch(20% 0.012 260deg)"
  night-ink: "oklch(95% 0.005 80deg)"
typography:
  display:
    fontFamily: "'Playfair Display', georgia, 'Times New Roman', serif"
    fontSize: "24px"
    fontWeight: 700
    lineHeight: 1.25
    letterSpacing: "-0.5px"
  title:
    fontFamily: "'DM Sans', -apple-system, blinkmacsystemfont, 'Segoe UI', system-ui, sans-serif"
    fontSize: "20px"
    fontWeight: 600
    lineHeight: 1.5
  body:
    fontFamily: "'DM Sans', -apple-system, blinkmacsystemfont, 'Segoe UI', system-ui, sans-serif"
    fontSize: "14px"
    fontWeight: 400
    lineHeight: 1.5
  label:
    fontFamily: "'DM Sans', -apple-system, blinkmacsystemfont, 'Segoe UI', system-ui, sans-serif"
    fontSize: "13px"
    fontWeight: 500
    letterSpacing: "0.05em"
rounded:
  sm: "4px"
  md: "6px"
  lg: "8px"
  full: "9999px"
spacing:
  "1": "4px"
  "2": "8px"
  "3": "12px"
  "4": "16px"
  "5": "20px"
  "6": "24px"
  "8": "32px"
  "10": "40px"
  "12": "48px"
  "16": "64px"
  "20": "80px"
  "24": "96px"
components:
  button-primary:
    backgroundColor: "{colors.slate}"
    textColor: "{colors.on-slate}"
    rounded: "{rounded.sm}"
    padding: "10px 20px"
  button-primary-hover:
    backgroundColor: "{colors.slate-deep}"
  button-secondary:
    backgroundColor: "{colors.paper-2}"
    textColor: "{colors.ink}"
    rounded: "{rounded.sm}"
    padding: "10px 20px"
  button-ghost:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.ink}"
    rounded: "{rounded.md}"
    padding: "8px 12px"
  button-glass-icon:
    backgroundColor: "{colors.veil}"
    textColor: "{colors.surface-raised}"
    rounded: "{rounded.full}"
    size: "40px"
  card:
    backgroundColor: "{colors.surface}"
    rounded: "{rounded.md}"
  nav-item:
    textColor: "{colors.ink-2}"
    rounded: "{rounded.md}"
    padding: "12px 16px"
  nav-item-active:
    backgroundColor: "{colors.slate}"
    textColor: "{colors.on-slate}"
  input-search:
    backgroundColor: "{colors.paper}"
    textColor: "{colors.ink}"
    rounded: "{rounded.md}"
    padding: "12px 16px"
  chip-toggle:
    textColor: "{colors.ink-2}"
    rounded: "{rounded.full}"
    padding: "0 12px"
    height: "32px"
---

# Design System: TurboPix

## Overview

**Creative North Star: "The Contact Sheet"**

The interface is a contact sheet on a light table: every photo gets the same quiet frame, the grid is the reading surface, and nothing in the chrome competes with the pictures. Browsing is scanning a proof sheet — calm, spacious, understated — and the only marks on the paper are the user's own: favorites in warm brass, selections and focus in deep slate.

Two materials carry the system. **Matte paper** — warm off-white surfaces, 1px hairlines at half opacity, ambient shadows — holds the content. **Layered glass** — translucent, blurred chrome that floats above (header, sidebar, control bars, dialogs, toasts) — carries the interface. Light mode is daylight paper (canvas at oklch 97.5%, hue 80); dark mode is the same room with the lights off (canvas at 14%) where the photos glow.

Type keeps the print register: Playfair Display for identity and section headings, DM Sans for everything functional, on a 4px rhythm. Motion is a courtesy, not a performance: three durations on one easing family (0.15 / 0.3 / 0.5s, cubic-bezier(0.4, 0, 0.2, 1)), one spring for view transitions and the indexing orbit, and both reduced-motion and reduced-transparency fallbacks built in.

**Key Characteristics:**
- Warm paper surfaces, deep slate interaction, exactly one brass mark.
- Two materials only: matte paper and layered glass.
- Contact-sheet grid: equal frames, generous gutters, chrome recedes.
- Editorial serif for identity; a quiet sans for every UI string.
- Motion 0.15/0.3/0.5s, one easing family, fully degradable.

## Colors

A warm, low-chroma palette: paper-light neutrals, one muted blue for interaction, one gold reserved for marks — nothing else competes.

### Primary

- **Deep Slate** (oklch(55% 0.08 250deg)): the interaction color. Fills active nav items and primary buttons, draws selection outlines, focus rings, and the timeline's translucent bars (mixed at 12–45% over surfaces).
- **Slate Deep** (oklch(45% 0.08 250deg)): hover / pressed state for slate fills.
- **Slate Soft** (oklch(65% 0.06 250deg)): the hover border on photo cards.

### Secondary

- **Warm Brass** (oklch(75% 0.12 80deg)): the grease-pencil mark. Fills the favorite icon when active and the rare attention button; it never fills a large surface.

### Tertiary

- **Signal Red** (oklch(55% 0.2 25deg), hover oklch(48% 0.2 25deg)): destructive actions (delete) and error borders.
- **Signal Green** (oklch(65% 0.18 155deg), hover oklch(58% 0.18 155deg)): confirming actions (keep, accept collage).

### Neutral

- **Paper** (oklch(97.5% 0.005 80deg)): the app canvas; inputs rest on it.
- **Paper Shade** (oklch(96% 0.005 80deg)): secondary surfaces — hover fills, meta blocks, secondary buttons.
- **Surface** (oklch(99% 0.002 80deg)): cards and resting bars.
- **Raised Surface** (oklch(100% 0 0deg)): the one pure-white layer — dialogs and inputs lifted above paper.
- **Ink** (oklch(20% 0.01 260deg)): primary text; **Ink Soft** (oklch(45% 0.01 260deg)): secondary text and resting nav labels; **Ink Faint** (oklch(50% 0.01 260deg)): muted meta — dark theme re-points it to oklch(65% 0.01 260deg), where the dark background needs the lighter end.
- **Slate Ink** (oklch(45% 0.08 250deg), dark oklch(80% 0.06 250deg)): text that sits on slate-tinted surfaces — pressed granularity pills, filtered timeline chips.
- **Danger Ink** (oklch(45% 0.2 25deg), dark oklch(80% 0.1 25deg)): error text on the 10% danger tint.
- **Hairline** (oklch(90% 0.005 260deg / 50%)): every 1px divider and border.
- **Glass** (oklch(99% 0.002 80deg / 70%) with a oklch(0% 0 0deg / 6%) hairline): the floating chrome material — header, sidebar, control bars, toasts, glass dialogs.
- **Veil** (oklch(20% 0.01 260deg / 45%), hover 65%): the scrim under icon buttons that sit on photos.
- Dark mode flips the neutral story: canvas oklch(14% 0.01 260deg), surface oklch(20% 0.012 260deg), text oklch(95% 0.005 80deg); slate and brass stay as they are.

### Named Rules

**The Grease-Pencil Rule.** Warm Brass marks only human actions and rare attention — favorites, a single retry. Keep it to small areas; its scarcity is the point.

**The Warm Paper Rule.** Neutral surfaces stay warm (hue 80, chroma ≤ 0.005) and text stays cool (hue 260). Never substitute pure gray for paper or a warm hue for ink.

## Typography

**Display Font:** Playfair Display (with georgia, 'Times New Roman', serif)
**Body Font:** DM Sans (with -apple-system, blinkmacsystemfont, 'Segoe UI', system-ui, sans-serif)
**Label/Mono Font:** JetBrains Mono is declared as a token but ships no web font — do not rely on it.

**Character:** An editorial serif for the name and the section tops, a geometric-humanist sans for every functional string. The pairing reads as print, not as software: quiet, spaced, decisive.

### Hierarchy

- **Display** (700, 24px, 1.25, -0.5px): the logo and section headings — the only place the serif speaks.
- **Title** (600, 20px): dialog titles; error-state headings step to 18px; meta section titles run 16px/700 uppercase with 0.8px tracking.
- **Body** (400, 14px, 1.5): the default UI voice; secondary copy drops to 13px, micro labels to 12px.
- **Label** (500, 13px, 0.05em, uppercase): sidebar section captions; card meta uses 13px sentence case.
- Sizes come from a fixed ramp (12 / 13 / 14 / 15 / 16 / 18 / 20 / 24 / 28, plus one 48px icon glyph) in weights 400 / 500 / 600 / 700.

### Named Rules

**The Two-Voice Rule.** Playfair Display speaks for identity and section headings only; every functional string is DM Sans.

## Layout

Fixed chrome over a single scrolling canvas. The header is 64px tall, full width; the sidebar is fixed left at 280px and thins to 240px below 1200px and 200px below 1024px; the main content is offset by the sidebar width and declares `container-type: inline-size`, so the grid adapts to its container, not the viewport.

The contact-sheet grid: `repeat(auto-fill, minmax(280px, 1fr))` with 24px gutters; at ≥1400px frames grow to 320px minimum with 32px gutters; below 1200px 240px/20px, below 1024px 200px/16px. On mobile the grid becomes a borderless three-column mosaic — 4px gutters, zero radius, cards flush — tightening to 2px below 480px.

Spacing is a 4px scale (4 → 96px). Page rhythm: 24px between sections, 32px before major blocks; content headers sit 24px above the grid; the selection bar floats 16px above the bottom edge. The viewer is a full-viewport grid (`'main sidebar' 1fr auto`); on mobile the sidebar becomes a 65vh bottom sheet and action bars reserve geometry for the native video control strip.

Breakpoints: 480 / 768 / 1024 / 1200 / 1400px viewport widths, plus container queries at ≤768 and ≤480px for the grid.

## Elevation & Depth

**Layered glass.** Depth comes from two materials, not from shadows. Paper surfaces are flat at rest and separated by 1px hairlines; glass chrome floats above content with `backdrop-filter: blur(16px) saturate(1.5)` (icon buttons use `blur(8px) saturate(1.5)`). The viewer's scrim is the deepest layer: oklch(0% 0 0deg / 80%) with `blur(12px) saturate(1.5)`.

### Shadow Vocabulary

- **--shadow-light** (`0 1px 2px oklch(0% 0 0deg / 4%), 0 1px 1px oklch(0% 0 0deg / 3%)`): resting cards, header, suggestion dropdowns.
- **--shadow-medium** (`0 2px 8px oklch(0% 0 0deg / 6%), 0 1px 3px oklch(0% 0 0deg / 4%)`): floating controls and bars; hover lift on buttons.
- **--shadow-heavy** (`0 4px 16px oklch(0% 0 0deg / 8%), 0 2px 6px oklch(0% 0 0deg / 5%)`): hover lift on cards and circular action buttons.
- **--shadow-glow**: defined but unused — reserved, not part of the vocabulary. Dark mode deepens all three steps rather than adding new ones.

### Named Rules

**The One-Step Lift Rule.** Hover moves a surface exactly one shadow step — light→heavy on cards, veil→veil-deep on glass icon buttons — with a scale of at most 1.15 on round controls.

**The Solid Fallback Rule.** Every glass surface must degrade to a solid paper surface under `prefers-reduced-transparency` or missing `backdrop-filter` (the scrim deepens to 92% black).

## Shapes

A four-step radius scale, assigned by rank: **4px** for compact primitives (filled buttons, inputs, small chips), **6px** as the default surface radius (cards, nav items, bars, search field), **8px** for panels (dialogs, viewer loading, selection bar), and **full** for pills, icon buttons, badges, and dots. Circles dominate the tool layer: 56px card action buttons, 48px viewer navigation, 40px zoom controls, 28px selection badges, 24px timeline handles.

Borders are 1px hairlines at half-opacity ink — never solid lines — with one dashed treatment reserved for "new" affordances (create album). The mobile mosaic drops radius to 0 so frames read as one sheet.

### Named Rules

**The Four-Step Radius Rule.** Radii come from 4 / 6 / 8 / full and nothing else; the 2px progress track is legacy drift, not a scale.

## Components

### Buttons

- **Shape:** 4px radius for filled buttons, 6px for outline buttons, full for icon buttons (40–56px circles).
- **Primary:** filled Deep Slate with white text, padding 10px 20px, 14px/500; hover steps to Slate Deep with a 1px lift and `--shadow-medium`.
- **Ghost / Outline:** Surface fill, 1px hairline border, 8px 12px padding; hover and focus only recolor the border to Deep Slate.
- **Icon (on photos/glass):** Veil background, `blur(8px) saturate(1.5)`, full radius; hover deepens the veil and scales ≤1.1; active state fills Deep Slate.
- **Circular card actions:** 56px, Surface fill, hairline, `--shadow-medium`; hover fills Deep Slate, scales 1.15, lifts to `--shadow-heavy`; the favorite turns Warm Brass when active.
- **Disabled:** opacity 0.5 with `cursor: not-allowed` (busy states use `wait`); destructive actions fill Signal Red, confirming actions Signal Green.
- **Motion:** "Softly tactile" — 0.15s color/transform transitions; feedback is visible but never bouncy.

### Cards

- **Corner Style:** 6px (0 on the mobile mosaic).
- **Background:** Surface; hairline border; `--shadow-light` at rest.
- **Hover:** shadow steps to heavy, border turns Slate Soft, the image scales 1.08 with a brightness/saturation nudge, and a bottom gradient overlay (70% black, `blur(1px)`) fades in title and meta — the caption band clears the 56px action buttons.
- **Internal Padding:** 20px inside the overlay; actions float 8px above the card edge.
- **Touch:** pointer-touch sees the overlay and actions permanently; on the phone mosaic (container ≤768px) the frame drops its radius and the caption plus 44px actions sit **below** the tile, and hover transforms are disabled.

### Inputs / Fields

- **Style:** Search field rests on Paper with a 1px hairline and 6px radius (padding 12px 16px, 16px text); modal inputs use Raised Surface or Paper Shade with a 4px radius.
- **Focus:** border turns Deep Slate; the modal variant adds a `0 0 0 3px oklch(55% 0.08 250deg / 10%)` ring; the search field also brightens to Surface.
- **Error:** inline box with 10% red fill, 30% red border, 4px radius.

### Dialogs

- **Two legitimate flavors:** native `<dialog>` forms (surface fill, 8px radius, 40% black backdrop) and the glass modal (Glass fill, `blur(16px)`, 60% black backdrop, `modal-entrance` 0.3s ease-out).
- **Structure:** 24px padding, hairline-divided header and action footer; actions right-aligned with 12px gaps.

### Toasts

- **Style:** bottom-center glass pill — Glass fill, `blur(16px) saturate(1.5)`, 6px radius, padding 12px 20px, deep soft shadow (`--shadow-heavy`).
- **Variants:** the leading icon carries the type (error red, success green, warning amber, info slate).
- **Motion:** 0.3s ease-out in, 0.3s ease-in out; auto-dismiss 2–4s.

### Navigation

- **Text-first:** sidebar items are label-only buttons at 15px, padding 12px 16px, 6px radius; hover tints Paper Shade and darkens the label; the active item fills Deep Slate with white 500-weight text.
- **Header:** 64px glass bar with a hairline-bottom and `--shadow-light`; the logo is Playfair at 24px.
- **Mobile:** the sidebar slides in as a drawer over a 40% black scrim; the header collapses to icon buttons at 44px.

### Chips & Pills

- **Style:** 32px tall, full radius, 1px hairline, transparent fill, 12px label.
- **States:** hover outlines and labels in Deep Slate; pressed fills a 12% slate mix with Slate Deep text; focus uses the double ring `0 0 0 2px surface, 0 0 0 4px slate`.
- **Use:** timeline granularity, filter labels, status chips.

### Viewer Chrome (signature)

Glass control bar (`blur(16px) saturate(1.5)`, 6px radius, 8px padding, 40px fade edges) floating over a blurred scrim, with veil icon buttons inside; on mobile the bar sits one step higher and buttons grow to 44px.

### Timeline Lane (signature)

Rendered once the library spans more than two distinct years — below that a grid is not a graph. A 56px lane where each period is a translucent slate bar (45% rest, 60% hover, 75% active; empty periods keep a 2px hairline bar), the selection is a 14% slate fill with a 1px slate border, and the 24px handles are its grab targets.

## Do's and Don'ts

### Do:

- **Do** let photos lead: the contact-sheet grid is the primary surface; chrome recedes behind paper tones and 1px hairlines.
- **Do** keep interaction singular: Deep Slate for active, selected, and focus states, mixed translucently (12–45%) for large areas.
- **Do** reserve Warm Brass for marks a person makes (favorites) and rare attention moments — small areas only.
- **Do** pair every glass surface with its solid fallback for reduced transparency and missing `backdrop-filter` support.
- **Do** build on the 4px spacing scale and keep ≥44px touch targets in viewer and mobile chrome.
- **Do** keep Playfair Display for identity headings only; all UI strings use DM Sans.
- **Do** use feather icons only (16–24px inline, no emoji).
- **Do** honor `prefers-reduced-motion` and `prefers-reduced-transparency` in every new effect.

### Don't:

- **Don't** fill large surfaces with Warm Brass or introduce new saturated hues.
- **Don't** add shadows beyond light/medium/heavy, and never reference tokens that don't exist (an undefined `--shadow-lg` reference once silently removed the dialogs' shadow).
- **Don't** use radius values outside 4/6/8/full — the 2px progress track is legacy drift.
- **Don't** hardcode hex or rgb colors in new components — error boxes and toast accents come from the danger/success/accent tokens.
- **Don't** use pure gray surfaces or warm text; paper stays warm (hue 80), ink stays cool (hue 260).
- **Don't** rely on `--font-mono`: the JetBrains Mono token ships no web font.
- **Don't** ship interactive controls without a visible `:focus-visible` treatment.
- **Don't** use emoji or non-feather icon sets.

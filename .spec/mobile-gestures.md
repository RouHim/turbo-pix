# Mobile gestures and animation polish

## Scope

Mobile web gestures only (`max-width: 768px`, touch input). Desktop layout, the
desktop sidebar, the desktop metadata column's side-panel presentation and all
mouse interactions stay unchanged. The Escape order and the closed-panel
`inert` state are not gestures and apply to both the mobile sheet and the
desktop column, because they are the same component. Gestures are additive:
every action keeps its visible control.

## Decisions

- **Drawer trigger:** horizontal swipe on the content surface; excluded are the
  map, timeline lane, the date rail, form fields and horizontal sliders. Swipe
  right opens the app drawer, swipe left on the drawer/scrim closes it.
  Disabled while selection mode is active, a modal/dialog is open, the mobile
  search panel is open, or the viewer is open.
- **Drawer feel:** the drawer and scrim track the finger one-to-one; release
  snap is decided by 30 % of the drawer width or 0.3 px/ms velocity. Opening
  never changes the persisted desktop `sidebarCollapsed` preference.
- **Details sheet:** the existing mobile `.viewer-sidebar` bottom sheet (65vh,
  one detent) is the details surface. An upward swipe on the unzoomed media
  opens it; a downward swipe on the photo area, the sheet handle/header, or
  the sheet body at `scrollTop === 0` closes it. The sheet body scrolls
  normally otherwise.
- **Dismiss priority:** with the sheet open, a downward swipe closes only the
  sheet; the viewer's pull-down dismiss applies only while the sheet is closed.
  Escape follows the same order.
- **Tap outside:** tapping the photo or empty viewer area while the sheet is
  open closes the sheet, not the viewer.
- **Horizontal navigation:** swiping left/right while the sheet is open keeps
  the sheet open and updates its content.
- **A11y:** the closed sheet is `inert` + `aria-hidden`; closing it returns
  focus to the metadata button when focus was inside it. Reduced motion
  replaces spatial animation with an immediate state change; direct
  manipulation still tracks the finger.
- **Motion:** only transform/opacity; drawer 0.3 s, sheet 0.3 s, both on the
  design-system easing. No new i18n strings, no new dependencies.

## Swipe animation repair

The horizontal viewer slide must have exactly one transform owner:

- `transition: none` (`.swiping`) stays on the active media through the drag
  and the settle animation; it is removed only after the new media is in
  place.
- The incoming neighbour is offset by the `.viewer-main` width, not
  `window.innerWidth`; unavailable neighbours remove `src` instead of setting
  an empty string.
- The adjacent preview stays visible until the new active media has loaded, so
  a slow decode cannot blank the slide. New touch gestures are ignored while
  that swap is in flight.
- Video neighbours use the existing `thumbnail?size=medium` URL.
- A media `reset()` that lands during an active drag/slide does not yank the
  track back under the finger.
- Touch cancel clears drag/pinch state and the media transition override.
- Reduced motion settles instantly (duration 0) for horizontal slide,
  snap-back, vertical dismiss and sheet snap.

## Acceptance criteria

1. On a mobile viewport, a right swipe on the photo grid opens the drawer; a
   left swipe on the drawer closes it. The map and selection mode ignore the
   gesture.
2. In the viewer, an up swipe on the photo opens the details sheet; a down
   swipe on the photo/sheet closes the sheet first and leaves the viewer open.
3. The existing down-swipe viewer dismiss still works with the sheet closed.
4. Horizontal swipe navigation remains smooth (no double-animation
   overshoot), and the viewer does not blank while the next photo loads.
5. Sheet navigation keeps the sheet open; the metadata updates.
6. Closed sheet descendants are not focusable; focus returns to the info
   button after closing.
7. The gestures work through the browser's real input pipeline (CDP
   `Input.dispatchTouchEvent`), not only through synthetic DOM `TouchEvent`s.
8. `npm run lint`, `npm run format:check`, `npm run test:unit`,
   `npm run test:i18n`, `cargo fmt --check`, `cargo clippy --all-targets -- -D
warnings`, the new E2E specs and the existing viewer/sidebar E2E specs pass.

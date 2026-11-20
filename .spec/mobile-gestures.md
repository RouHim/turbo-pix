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
- **Video seek:** a double-tap in the left/right third seeks −10 s/+10 s and
  preserves the play/pause state; the middle third is inert because the native
  controls own it. A transient `−10 s`/`+10 s` status appears for 0.7 s. The
  control strip's bottom 48 px never seeks.
- **Chrome toggle:** a single tap on the photo (mobile, non-video) toggles the
  close/prev/next buttons and the bottom control bar. Hidden chrome is
  `inert`; any tap while hidden restores it and does not close the sheet or
  viewer. With chrome visible, the existing order applies: sheet first, then
  viewer. The details sheet remains visible and independently closable.
- **Range selection:** after a long-press enters selection, dragging over
  cards extends a linear, display-order range from the pressed card to the
  card under the finger; moving back shrinks it. It works on the photo grid,
  collages and housekeeping cards; the nearest scroll container auto-scrolls
  at its 48 px edges. The release keeps the selection; the gesture suppresses
  clicks and the drawer.
- **Sheet header navigation:** a horizontal swipe on the visible
  `.photo-info-header` (not the scrolling body) navigates to the
  previous/next photo with the same slide as the media gesture, keeps the
  sheet open, and updates its metadata. The 24 px axis lock prevents vertical
  sheet drags from triggering it.
- **Search panel:** the mobile search surface is a fixed header bar, not a
  sheet; it deliberately has no swipe-down dismissal. The header search button
  remains the toggle.
- **A11y:** the closed sheet is `inert` + `aria-hidden`; closing it returns
  focus to the metadata button when focus was inside it. Hidden viewer chrome
  is also `inert`; focus moves to the viewer if it was inside the chrome. The
  seek feedback is a polite status. Reduced motion replaces spatial animation
  with an immediate state change; direct manipulation still tracks the finger.
- **Motion:** only transform/opacity; drawer 0.3 s, sheet 0.3 s, both on the
  design-system easing. Only the two seek labels are new i18n keys; no new
  dependencies.

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
9. A video double-tap in the left/right third seeks and announces the matching
   step; a middle-third double-tap does not seek.
10. Tapping the photo hides chrome; tapping again restores it. While chrome is
    hidden, any tap restores it before the sheet or viewer can close.
11. Long-press+drag selects a shrinking range on the photo, collage and
    housekeeping surfaces; the drawer and the post-release click are
    suppressed.
12. A horizontal swipe on the details header navigates and keeps the sheet
    open.
13. The mobile search bar does not claim a vertical dismiss gesture.

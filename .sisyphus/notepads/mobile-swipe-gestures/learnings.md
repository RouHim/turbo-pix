# Learnings — mobile-swipe-gestures

## Plan Overview
- 7 implementation tasks + 4 final verification tasks
- Wave 1: Tasks 1, 2, 3 (all parallel)
- Wave 2: Task 4 (then 5+6 in parallel)
- Wave 3: Task 7
- Final: F1-F4

## Key Constraints
- NO animation libraries (GSAP, Framer Motion) — use CSS transforms + requestAnimationFrame
- NO changes to viewerControls.js zoom/pan mechanics (only ADD isAtPanBoundary() in Task 6)
- NO touch-action on body/main/sidebar — only on viewer elements
- NO timeout-based disambiguation — use directional thresholds (dx vs dy)
- NO new Playwright mobile projects — use existing desktop project with viewport resize
- NO spring physics engine — match existing friction model (0.95) from viewerControls.js
- NO haptic feedback changes
- NO sidebar/bottom sheet/grid/video gestures

## CSS Gesture Notes
- Add `touch-action: none` only within the viewer subtree (`.viewer-overlay`, `.viewer-main`)
- Pair `overscroll-behavior: contain` on `.photo-viewer` with `touch-action: none` to prevent pull-to-refresh/back-swipe conflicts
- Disable selection/callout on `.viewer-image` and `.viewer-video` to keep swipe gestures clean on touch devices

## Animation Targets
- Swipe: 300ms ease-out snap-back; photo follows finger at 1:1 translateX
- Dismiss: 250ms ease-in (falling away feeling)
- Rubber-band: 0.3 resistance factor; 400ms spring timing on release
- Velocity threshold: 0.3px/ms horizontal, 0.5px/ms vertical
- Distance threshold: 30% viewport width (horizontal), 150px (vertical)

## Architecture Notes
- gestureManager.js has BROKEN inline swipe logic (lines 304-343) — bypasses SwipeRecognizer
- SwipeRecognizer, LongPressRecognizer instantiated but recognize() NEVER called
- touchHandler in utils.js is dead code (never imported anywhere)
- Velocity calculation is last-frame-only (needs averaging over 3-5 frames)
- Pan-vs-swipe disambiguation broken (enablePan/disablePan just set activeGesture)
- ZERO touch-action CSS properties in entire codebase

## Cleanup Notes
- Removed dead `touchHandler` and `LongPressRecognizer` exports after confirming no remaining references in `static/`.
- `npm run lint` stayed clean after the deletions.

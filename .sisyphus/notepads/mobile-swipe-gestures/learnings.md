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

## GestureManager Refactor Notes
- `GestureManager` can preserve the viewer callback contract by delegating final swipe/tap decisions to recognizers and forwarding `recognizer.data` unchanged to `.on('swipe')` and `.on('doubleTap')` callbacks.
- Smoothing release momentum works well by averaging the last up-to-5 `touchmove` velocity samples, then reusing that averaged velocity for both swipe recognition and `panEnd` momentum.
- Axis lock is safest when it is decided once after movement exceeds 10px and then applied only to gesture output/recognition, not to the stored raw touch coordinates.

## Task 4 Swipe Viewer Notes
- `SwipeableViewer` works best as a viewer-local visual layer: keep threshold/RAF animation/adjacent-image rendering in `viewer.js`, and let `ViewerControls` stay focused on zoom/pan math.
- Horizontal viewer swipe needs `GestureManager.enablePan()` from `touchmove` after axis intent is clear; enabling pan immediately on `touchstart` would steal vertical gestures like swipe-down-to-close before axis lock.
- Reset swipe transforms on every media swap/error path (`showImage`, `setVideoSource`, `showError`, `close`) so interrupted swipe animations never leak into the next photo.

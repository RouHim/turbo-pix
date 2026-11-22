# Coding Standards

Reviewer-facing rules for judgement calls that no linter can enforce.

## Interaction tests

- Bounded interactions (seek, zoom, panel settle, range selection) MUST assert
  the exact target value or geometry and the no-op zone. A direction-only or
  greater-than assertion is insufficient: it cannot fail when an effect is
  applied twice.
- A custom gesture layered on a native control surface (`<video controls>`, a
  scrollable sheet, a native slider) MUST explicitly own the interaction and
  suppress the native action, or prove that the native path is the one under
  test. Cover it with real input-pipeline events (CDP), not synthetic DOM
  events alone.

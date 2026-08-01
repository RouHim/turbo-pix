# Feature Specification: Color-Blind Mode

**Intent**: enhancement
**Complexity**: Tier 2
**Created**: 2026-05-24

---

## Problem Statement

Approximately 8% of males and 0.5% of females have some form of color vision deficiency (CVD), with red-green deficiency (deuteranopia/protanopia) accounting for ~99% of cases. TurboPix currently relies on color alone to convey state in several places — success/error indicators, active/inactive navigation, favorite badges, and status labels. Users with CVD cannot distinguish these cues, making the application harder or impossible to use effectively.

There is no existing accessibility accommodation. A color-blind mode would remap the UI color palette to use CVD-safe color combinations while preserving the existing light/dark theme architecture.

## User Scenarios & Testing

### User Story 1 — Enable Color-Blind Mode (Priority: P1)

As a user with color vision deficiency, I want to activate a color-blind friendly palette so that I can distinguish all UI states and indicators clearly.

**Acceptance Scenarios**:

1. **Given** a user with red-green color blindness opens TurboPix for the first time, **When** they navigate to the appearance settings, **Then** they see an option to enable "Color-Blind Mode" with profile choices (at minimum: red-green / deuteranopia-protanopia).

2. **Given** color-blind mode is enabled, **When** the user views the photo grid, **Then** favorite badges (heart icon states), selection indicators, and status labels use colors distinguishable by red-green CVD users — e.g., blue/orange instead of red/green.

3. **Given** color-blind mode is enabled and the user switches between light and dark themes, **When** they toggle the theme, **Then** the color-blind palette adapts appropriately — maintaining distinguishability in both light and dark contexts.

4. **Given** color-blind mode is enabled, **When** the user closes and reopens the browser, **Then** the setting is persisted and the color-blind palette is still active.

5. **Given** a user has enabled color-blind mode, **When** they view error messages, success toasts, or warning indicators, **Then** each state is distinguishable from the others without relying on red/green hue perception.

---

## Functional Requirements

- **FR-001**: The application SHALL provide a user-facing toggle to enable/disable color-blind mode, accessible from a settings or appearance section of the UI.

- **FR-002**: Color-blind mode SHALL remap the application's color palette to use color combinations that are distinguishable by users with red-green color vision deficiency (deuteranopia and protanopia).

- **FR-003**: The color-blind palette SHALL maintain semantic meaning — success, error, warning, info, active, and inactive states must each have visually distinct colors.

- **FR-004**: Color-blind mode SHALL be compatible with both light and dark themes, providing appropriate palette variants for each.

- **FR-005**: The user's color-blind mode preference SHALL be persisted across browser sessions (e.g., via `localStorage`).

---

## Success Criteria

- **SC-001**: All status indicators (success, error, warning, info, active, inactive) are distinguishable from one another when viewed by users with red-green color blindness — verified by deuteranopia/protanopia simulation testing.

- **SC-002**: A color-blind user can complete the core task flow (browse photos → search → open viewer → toggle favorite → navigate to favorites view) without encountering a UI element that relies solely on red/green distinction.

---

## Assumptions

- v1 targets red-green deficiency (deuteranopia/protanopia) only, as it affects the vast majority of color-blind individuals. Tritanopia (blue-yellow) and achromatopsia (complete) are out of scope for this iteration.
- Only the UI chrome (buttons, badges, indicators, text, borders) is affected — photo content itself is NOT modified or filtered.
- The existing CSS custom property (`@layer tokens`) architecture is sufficient; no individual component rewrites are needed — only the token values change.
- Users self-identify their color vision needs and manually enable the mode. No automatic detection via browser media queries is attempted in v1.
- The mode toggle lives alongside the existing theme toggle in the header or a nearby settings area.

---

## Key Entities

- **Color Profile**: A named set of CSS custom property overrides (e.g., "deuteranopia-safe") that remap semantic color tokens to CVD-distinguishable equivalents while preserving light/dark variants.

---

## Edge Cases

- User enables color-blind mode, then switches OS-level high-contrast or forced-colors mode — the two must not conflict (color-blind palette should defer to OS forced-colors if active).
- User has color-blind mode enabled, clears browser storage — the setting resets to default (off) gracefully.
- Future: if additional CVD profiles are added (tritanopia, achromatopsia), the UI must support selecting between multiple profiles without confusion.

---

## Out of Scope

- Photo/image color correction or daltonization filters applied to viewed content.
- Automatic detection of color vision deficiency via browser media queries or OS settings.
- Tritanopia (blue-yellow) and achromatopsia (complete color blindness) profiles.
- High-contrast mode (separate accessibility feature).

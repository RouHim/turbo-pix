# Feature Specification: Photo Place Names and In-Panel Mini Map

**Created**: 2026-11-24
**Status**: Approved
**Input**: Viewer detail panel shows raw GPS only; resolve the place name offline and show where the photo was taken on a small map.

## Goal

The viewer's metadata panel already lists a photo's coordinates but not the place they belong to. This feature resolves coordinates into a place name **entirely offline** — a GeoNames-derived dataset is embedded into the binary at build time, so no third-party geocoding service is contacted at scan time or at view time — and shows that name plus a small, non-interactive map in the Location section. The place name is a first-class, filterable value; the mini map hands off to the existing Map view focused on that photo.

Out of scope: editing place names, street/address-level resolution, self-hosted map tiles, dataset auto-update, place names on grid cards.

## User Scenarios

### Scenario 1 - See where a photo was taken (P1)

The user opens a photo and reads the resolved place next to its coordinates.

**Acceptance**
1. Given a photo with coordinates and a resolved place, When the viewer's detail panel is shown, Then the Location section lists the place name (for example `Volksdorf, Hamburg`) and, when the dataset carries one, the country name in the UI's language (for example `Deutschland`, `Germany`), above the existing GPS coordinates.
2. Given a photo whose coordinates resolved to nothing (no place within range), When the panel is shown, Then the Location section still shows the GPS coordinates without an empty place row.
3. Given a photo without coordinates and without a resolved place, When the panel is shown, Then no Location section is rendered, exactly as today.
4. Given the place name is missing but coordinates exist, When any other surface labels the photo (map popup, marker label), Then it keeps falling back to the coordinates.

### Scenario 2 - Mini map and jump to the library map (P1)

The user sees where the photo is and can jump to the full map.

**Acceptance**
1. Given a photo with coordinates, When the Location section is shown, Then it renders a small map with a marker at the photo's coordinates, at neighbourhood zoom, with the required map attribution visible.
2. Given the mini map, When the user clicks it or activates the "Show on map" control, Then the app navigates to `/map?focus=<hash>` and the Map view centers on the photo's location, zoomed in, with that location's popup open.
3. Given the mini map, When the user drags, scrolls, or uses the wheel over it, Then the map does not pan or zoom: the gesture reaches the panel's scroll container, and the map is not a keyboard trap.
4. Given a photo URL that carries `?focus=<hash>` but the hash is not part of the plotted set, When the Map view loads, Then it fits the plotted set as it does without the parameter.
5. Given reduced-motion preferences, When the focus navigation happens, Then no animated pan or zoom is performed.

### Scenario 3 - Filter the library by place (P2)

The user turns a place name into a search.

**Acceptance**
1. Given a photo with a resolved place, When the user activates the place name, Then the viewer closes, the grid shows the results of `location:<place>` (the same prefix search the search bar uses), and the URL reflects it.
2. Given the resolved place contains a comma (a district plus its parent city), When the filter runs, Then the full place string is searched and the photos with that place are returned.

### Scenario 4 - Offline resolution and backfill (P1)

**Acceptance**
1. Given the binary is built, When the app indexes photos with coordinates, Then place names are computed in-process from the embedded dataset, with no network request and no rate limiting; the indexing phase completes in seconds for a large library.
2. Given a photo's coordinates are moved by a metadata save, When the save commits, Then the stored place and country are dropped and the photo is queued for resolution again (existing re-arm behaviour).
3. Given an installation whose rows still carry names from the previous (network) resolver, When the new migration runs, Then those names are cleared and every row is re-queued; the next scan rebuilds them from the dataset.
4. Given the dataset file is missing, When the project is built, Then the build stops with a message naming `./scripts/fetch-geodata.sh`.

### Scenario 5 - Degraded states (P2)

**Acceptance**
1. Given the tile endpoint is unreachable, When the mini map renders, Then the marker and the attribution stay visible on a plain background.
2. Given the place dataset resolves nothing within range, When the panel and map render, Then the coordinates are shown and no placeholder text appears.

## Functional Requirements

- **FR-001**: The viewer's Location section lists `metadata.location.place` when present, the country name derived from `metadata.location.country_code` in the active UI locale, and the existing six-decimal GPS pair.
- **FR-002**: Place resolution is offline: the process reads an embedded dataset, never a geocoding HTTP API. No configuration variable selects a geocoding service.
- **FR-003**: The dataset is derived from GeoNames `cities500` plus the `allCountries` `PPLX` rows, deduplicated by geonameid, projected to seven tab-separated columns behind a `#turbo-pix-places-v1` header, and validated (row count, column count, anchor row, UTF-8) by `scripts/fetch-geodata.sh`. It is embedded into the binary at build time.
- **FR-004**: Resolution picks the nearest place within 25 km. When that place is a `PPLX` district, the most populous parent within 30 km that shares the district's country code is appended as `<district>, <city>`; a parent in another country is never chosen.
- **FR-005**: The resolved values are stored as `metadata.location.place` (string) and `metadata.location.country_code` (ISO-3166 alpha-2); coordinates keep their derived columns and their nesting into `metadata.location` in API replies.
- **FR-006**: The `location:` search prefix and the general search match the resolved place, and a metadata save that moves the photo still clears the name and re-arms resolution.
- **FR-007**: A one-time migration clears names written by the previous resolver and re-arms every photo carrying coordinates.
- **FR-008**: The Location section renders when the photo has coordinates **or** a resolved place.
- **FR-009**: The mini map is a non-interactive Leaflet map at a fixed neighbourhood zoom with one marker, no zoom control, no drag/wheel/touch/keyboard panning, reduced-motion-safe, and shows OpenStreetMap attribution plus the place-dataset attribution.
- **FR-010**: The mini map and a labeled control navigate to `/map?focus=<hash>`; the Map view zooms to the photo's location at neighbourhood zoom (15) — raised only as far as a cluster around the target requires — and opens its popup. `focus` is only meaningful on the Map view and is applied once per hash, so a later filter change does not yank the viewport back.
- **FR-011**: Activating the place name navigates to the grid filtered by `location:<place>` and closes the viewer.
- **FR-012**: Every new user-visible string exists in English and German with no parity drift.
- **FR-013**: Country names are rendered from the ISO code with `Intl.DisplayNames` in the active locale; when the runtime cannot resolve a code, the code itself is shown.
- **FR-014**: The map attribution on every map surface credits both OpenStreetMap and GeoNames; README's licence section credits GeoNames (CC BY 4.0).

## Key Entities

- **Place**: a named populated place from the dataset (city, town, village, or `PPLX` district) with coordinates, country code, and population.
- **Resolved place**: the pair stored on a photo — `place` (composed label) and `country_code`.
- **Place dataset**: the derived, validated GeoNames file embedded into the binary.
- **Mini map**: the non-interactive map in the metadata panel.
- **Focus target**: the `?focus=<hash>` parameter that turns the Map view into a single-photo view.

## Edge Cases

- A photo in open water or wilderness with no place within 25 km keeps coordinates only.
- A `PPLX` district whose only nearby large city lies in another country resolves to the district name alone.
- Non-ASCII place names (`København`, `München`) round-trip through the dataset, the JSON metadata, and the API unchanged.
- Photos sharing identical coordinates share one place name; the map groups them as today.
- A place name containing a comma is searched as one `location:` phrase.
- The mobile details sheet renders the same section; the mini map must not steal the sheet's vertical swipe.
- An empty or malformed dataset row is skipped during indexing; a dataset without valid rows yields no names rather than failing the scan.

## Research Notes

- `this-week-in-past` (the user's prior implementation) resolves offline from the same GeoNames derivation: `cities500` ∪ `allCountries` PPLX, ~396 k rows / ~17 MB, 0.25° grid index, `<1 ms` lookups, district + most populous parent + country. Its known border bug (a parent city in another country within 30 km) is fixed here by requiring the same country code.
- GeoNames data is licensed CC BY 4.0; attribution is required.
- The repository already embeds build outputs (`build.rs` embeds `dist/`) and fetches build inputs with a pinned script (`scripts/fetch-ort.sh`), so an embedded dataset follows an established pattern.

## Assumptions

- The dataset is baked into release artifacts; a rebuild refreshes it. There is no runtime update mechanism and no second artifact to host.
- The 25 km / 30 km radii are fixed and not configurable.
- Country names are UI strings derived from a code, not stored strings; the dataset's own names are shown as-is.
- The mini map deliberately trades interaction for scroll safety; exploration happens in the Map view.
- The Map view's viewport stays out of the URL; only `focus` is added, and only on that view.

## Success Criteria

- **SC-001**: Opening a photo with coordinates in a built binary shows its place name with no network traffic to any geocoding service.
- **SC-002**: A library of 10 000 geo-located photos is fully resolved in one indexing pass in well under a minute on one CPU.
- **SC-003**: From the metadata panel, two interactions (mini map → marker popup) reach the same location in the Map view.
- **SC-004**: `location:<place>` from the panel returns exactly the photos stored with that place.
- **SC-005**: All new strings exist in both languages and the i18n integrity check passes.
- **SC-006**: Keyboard users can reach the place filter and the "Show on map" control; the mini map's canvas is not a keyboard stop, and its attribution links stay reachable.

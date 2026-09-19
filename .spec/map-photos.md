# Feature Specification: Map View — OpenStreetMap Photos Map

**Created**: 2026-09-19
**Status**: Approved
**Input**: Maps feature — show an OpenStreetMap map with the photos taken at each mapped location.

## Goal

The library already knows where each photo was taken (EXIF GPS plus manually corrected coordinates) but only exposes that as text in the viewer. This feature adds a top-level Map view that places the library on an OpenStreetMap basemap, honoring the same search and filter state as every other view. It is a read-only overview and navigation surface: markers locate photos, and a popup hands off to the existing full-screen viewer.

Out of scope: editing GPS by dragging markers, offline tile prefetching, new reverse-geocoding, heatmaps, or route/drawing tools.

## User Scenarios

### Scenario 1 - See where the library was photographed (P1)

The user opens the Map view and immediately sees a clustered overview of every photo in the current filter state placed on an OpenStreetMap basemap.

**Acceptance**
1. Given the library contains photos with GPS coordinates, When the user opens the Map view from the sidebar, Then the map renders OpenStreetMap tiles, the required attribution, and clustered markers covering every geo-located photo in the current filter state.
2. Given the library also contains photos without coordinates, When the Map view is displayed, Then those photos are not plotted and the view reports how many matching photos have no location data.
3. Given the app is opened directly at the Map view's URL, When the view loads, Then it behaves identically to selecting the view from the sidebar.

### Scenario 2 - Explore a location and open a photo (P1)

The user drills into a cluster, opens a marker popup, and jumps into the full-screen viewer.

**Acceptance**
1. Given a cluster marker, When the user activates it, Then the map zooms in and the cluster separates into smaller clusters and/or individual photo markers.
2. Given an individual photo marker, When the user activates it, Then a popup shows the photo's thumbnail and capture date, the number of photos at that location, and the place name (or the coordinates when no place name is known).
3. Given the popup is open, When the user activates a thumbnail, Then the existing viewer opens on that photo and next/previous navigation pages through the currently filtered and sorted photo set.
4. Given several photos share identical coordinates, When their marker is activated, Then the popup lists all of them in a scrollable list in the current sort order.

### Scenario 3 - The map follows the app's filters (P2)

The user narrows the library and expects the markers to match the grid.

**Acceptance**
1. Given the map is open, When the user applies a year/month filter, Then the plotted markers reduce to exactly the geo-located subset of the new result set without a page reload.
2. Given the user runs a search query such as `location:Berlin`, When results render, Then only the matching geo-located photos remain plotted.
3. Given the user navigates between views with browser back/forward, Then the map returns with the same filter state the rest of the app uses.
4. Given the user leaves the Map view and returns to it later, Then the map re-fits the current filtered photo set.

### Scenario 4 - Degraded and empty states (P2)

The map stays usable when there is nothing to plot, when names are missing, or when tiles cannot be fetched.

**Acceptance**
1. Given no photo in the current filter state has coordinates, When the Map view is displayed, Then it shows an explanatory empty state instead of a blank map.
2. Given the tile endpoint is unreachable or blocked, When the Map view is displayed, Then markers, popups and attribution still render on a plain background alongside a visible non-blocking notice.
3. Given a location has no resolved place name, When its popup is opened, Then the popup shows the coordinates.

## Functional Requirements

- **FR-001**: The application provides a Map view as a top-level view, selectable from the sidebar and addressable by URL, alongside the existing views.
- **FR-002**: The Map view renders OpenStreetMap raster tiles from a tile endpoint that is configurable on the running server and defaults to the public OpenStreetMap tile service; switching to a self-hosted or alternative OSM-compatible endpoint requires configuration only, not a code change or rebuild.
- **FR-003**: Wherever map tiles are displayed, the OpenStreetMap attribution ("© OpenStreetMap contributors", linking to the OpenStreetMap copyright page) is visible, not hidden behind a toggle, off-screen, or covered by controls.
- **FR-004**: The Map view applies the same filter state as the other views (search query, favorites, videos, album, year/month) and plots exactly the geo-located subset of the photos that the grid would show for that state.
- **FR-005**: Every photo in the current filtered set that has both latitude and longitude is represented on the map, independent of grid pagination.
- **FR-006**: Photos that are close together at the current zoom level are grouped into a cluster marker labeled with the number of photos it contains; zooming in splits clusters into smaller clusters and individual photo markers.
- **FR-007**: Activating a cluster zooms the map so that the cluster's photos separate.
- **FR-008**: Activating a photo marker opens an in-map popup for that location showing its photos (thumbnail and capture date), the photo count, and the resolved place name when one is available.
- **FR-009**: Photos sharing identical coordinates appear as a single marker whose popup lists all of them.
- **FR-010**: A location's popup lists all of its photos in the current sort order, scrollable when the list is long; the list is not truncated.
- **FR-011**: Activating a thumbnail in a popup opens the existing viewer on that photo, with next/previous navigation over the current filtered and sorted set — the same set the grid provides.
- **FR-012**: When a location has no resolved place name, its popup shows the coordinates instead.
- **FR-013**: The Map view reports how many photos in the current filtered set have no location data.
- **FR-014**: When no photo in the current filtered set has location data, the view shows an explanatory empty state.
- **FR-015**: When map tiles fail to load (offline, blocked, or unreachable endpoint), the view shows a non-blocking notice while still rendering markers, popups, and attribution.
- **FR-016**: Videos that carry coordinates are plotted like photos, and opening one from a popup plays it through the existing viewer behavior.
- **FR-017**: Entering the Map view re-fits the map to the current filtered photo set; the map viewport (center/zoom) is not stored in the URL.
- **FR-018**: The Map view is operable by keyboard alone: clusters, markers and popup thumbnails can be focused and activated, focus is visibly indicated, and reduced-motion preferences suppress animated panning and zooming.
- **FR-019**: The Map view is usable at mobile and desktop viewport sizes without changing existing sidebar or navigation behavior.
- **FR-020**: All new user-visible text is provided in both English and German, consistent with the existing translation system, and any new sidebar icon comes from the existing icon set.

## Key Entities

- **Geo-located photo**: a library item (image or video) identified by content hash, carrying a capture date, a thumbnail, coordinates from EXIF or manual metadata editing, and an optional resolved place name.
- **Map marker**: the visual representation of one photo at its coordinates.
- **Cluster**: a zoom-dependent grouping of nearby markers rendered as a single count bubble.
- **Tile endpoint**: the OSM-compatible raster tile source the map reads from, configurable by the operator.

## Edge Cases

- Photos with only one of the two coordinates, or out-of-range values, never reach the map — the library rejects such data at write time; only photos with both coordinates are plotted.
- Valid but implausible coordinates (for example 0,0) are plotted exactly as stored; the map does not second-guess library data.
- A single location holding a very large number of photos remains fully reachable through its scrollable popup list.
- Dense libraries (tens of thousands of geo-located photos) must keep panning, zooming and cluster interaction responsive.
- Photos at extreme latitudes or spanning the antimeridian render without breaking the map.
- Changing filters while the map is open updates markers in place, with no manual view switch required.
- A filter state with no geo-located matches shows the empty state while other views continue to show their own results.
- An unconfigured or unreachable tile endpoint degrades to the non-blocking notice state, never to a blank or frozen view.
- Browser back/forward navigation preserves filter state across views exactly as it does today.

## Research Notes

- https://operations.osmfoundation.org/policies/tiles/ — browser-side interactive viewing of `https://tile.openstreetmap.org/{z}/{x}/{y}.png` is permitted with visible attribution and a normal `Referer`, while bulk/prefetch/offline downloading is prohibited and hard-coding the tile URL is explicitly discouraged; the spec therefore requires a configurable tile endpoint, no prefetching, and always-visible attribution.
- https://wiki.osmfoundation.org/wiki/Licence/Attribution_Guidelines — the required credit is "© OpenStreetMap contributors" with a link to the copyright page, placed on the map itself.
- Repository evidence (`src/geo_location.rs`, `src/db.rs`) — the app already reverse-geocodes with OpenStreetMap's Nominatim and stores the resolved city, so popups reuse existing data instead of introducing a new external dependency.

## Assumptions

- The map is read-only: placing or dragging markers to change GPS coordinates is out of scope; existing viewer metadata editing remains the way to correct location data.
- Videos count as photos for map purposes.
- Photos without GPS are absent from the map and surfaced only as a count.
- The tile endpoint is provided by the running server, defaults to the public OpenStreetMap tile service, and can be repointed by configuration alone.
- The map viewport (center/zoom) is not part of the URL; re-entering the view re-fits the current filtered set.
- Place names come from location data the library has already resolved; no new reverse-geocoding is introduced.
- No heatmaps, drawing tools, timeline brush, marker color coding, or city-name grouping are included.
- Existing grid, viewer, sorting, selection, and metadata-editing behavior is unchanged.

## Success Criteria

- **SC-001**: With a library containing geo-located photos, opening the Map view shows a clustered overview of every matching geo-located photo, and the number of photos represented equals the number of geo-located photos in the grid's result set for the same filter state.
- **SC-002**: From the Map view, a user reaches the full-screen viewer for a specific photo in at most three interactions (marker → thumbnail → viewer), with next/previous working across the same filtered, sorted set the grid uses.
- **SC-003**: Changing the year/month filter or running a location search updates the plotted markers to exactly the geo-located subset of the new result set, without a page reload.
- **SC-004**: The map remains interactive (pan, zoom, cluster expansion, popup opening) with 10,000 geo-located photos in the library.
- **SC-005**: With the tile endpoint unreachable, the view still shows markers, popups and attribution plus a visible non-blocking notice — never a blank or broken map.
- **SC-006**: The OpenStreetMap attribution is visible whenever tiles are displayed, at both desktop and mobile viewport sizes.
- **SC-007**: All new user-visible strings exist in both English and German with no parity drift, and the i18n integrity check passes.
- **SC-008**: Every map interaction listed in the requirements (pan, zoom, cluster expansion, marker popup, thumbnail activation) is reachable by keyboard alone.

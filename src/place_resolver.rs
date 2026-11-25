//! Offline place resolution.
//!
//! Photo coordinates are turned into a human place label without any network
//! call: the GeoNames-derived dataset is embedded into the binary at build time
//! (`scripts/fetch-geodata.sh` writes it, `build.rs` refuses to build without
//! it) and parsed into a compact in-memory index the first time a photo needs a
//! name.
//!
//! The dataset is `cities500` plus the `allCountries` `PPLX` district rows,
//! deduplicated and projected to seven tab-separated columns behind a
//! `#turbo-pix-places-v1` header (~396k rows, ~17 MB). The index keeps 24 bytes
//! per row in a latitude/longitude grid with CSR cell buckets, because it is
//! held for the whole process lifetime.

use std::sync::OnceLock;

use log::warn;

/// Edge length of the lookup grid cells in degrees (0.25° ≈ 28 km at the equator).
///
/// Smaller cells shrink the bounding-box scan but grow [`PlaceIndex::cell_starts`]
/// quadratically: 0.25° costs ~4 MB of cell offsets.
const CELL_DEG: f64 = 0.25;

/// Mean Earth radius in kilometers, matching [`haversine_km`].
const EARTH_RADIUS_KM: f64 = 6371.0088;

/// Kilometers per degree of latitude on the sphere [`EARTH_RADIUS_KM`] describes.
///
/// Derived rather than hard-coded `111.32`: the candidate box must not be
/// narrower than the radius [`haversine_km`] measures, and a single source of
/// truth for the sphere keeps the two from drifting apart.
const KM_PER_DEG_LAT: f64 = EARTH_RADIUS_KM * std::f64::consts::PI / 180.0;

/// Slack added to the candidate-box spans, in degrees.
///
/// The box must contain every place the radius can reach, while stored
/// coordinates are `f32` (≤ 4e-6° rounding) and the span arithmetic itself
/// rounds. The margin covers both.
const BOX_SLACK_DEG: f64 = 1e-4;

/// Number of tab-separated columns in one derived dataset row.
const COLUMNS: usize = 7;

/// Column indices of the derived dataset, as written by `scripts/fetch-geodata.sh`.
const NAME_COLUMN: usize = 0;
const LATITUDE_COLUMN: usize = 1;
const LONGITUDE_COLUMN: usize = 2;
const FEATURE_CLASS_COLUMN: usize = 3;
const FEATURE_CODE_COLUMN: usize = 4;
const COUNTRY_COLUMN: usize = 5;
const POPULATION_COLUMN: usize = 6;

/// First field of the dataset's header line; files that do not start with it are
/// not the derived dataset and are refused instead of parsed column by column.
const HEADER_PREFIX: &str = "#turbo-pix-places-v1";

/// Observed average row size of the derived dataset; used only to pre-size
/// parser buffers so a 17 MB text does not reallocate repeatedly.
const BYTES_PER_ROW_ESTIMATE: usize = 44;

/// Average name length share of a row (the name column is all the arena stores).
const NAME_BYTES_PER_ROW_ESTIMATE: usize = 12;

/// Maximum distance from the query point to the nearest place for a valid match.
///
/// Beyond this the photo keeps its coordinates only: a 45 km-away town would
/// name a photo taken deep in the mountains after a city it never visited.
const MAX_DISTANCE_KM: f64 = 25.0;

/// Maximum distance from the query point to the parent city of a district.
const MAX_PARENT_DISTANCE_KM: f64 = 30.0;

/// What a dataset row can contribute to a label.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PlaceKind {
    /// Usable as a parent city: `P` rows with a populated-place code.
    Parent,
    /// District (`P` + `PPLX`): displayed as `<district>, <parent city>`.
    District,
    /// Anything else: neither district nor parent.
    Other,
}

/// One dataset row, packed to 24 bytes; `name` lives in [`PlaceIndex::names`].
#[derive(Clone, Copy, Debug)]
struct Entry {
    name_start: u32,
    name_len: u16,
    country: [u8; 2],
    kind: PlaceKind,
    lat: f32,
    lon: f32,
    population: u32,
}

/// A place resolved by a query, borrowed from the index it came from.
#[derive(Debug)]
struct Hit<'a> {
    name: &'a str,
    country_code: Option<&'a str>,
    kind: PlaceKind,
}

impl Hit<'_> {
    fn is_district(&self) -> bool {
        self.kind == PlaceKind::District
    }
}

/// The place label derived for a photo: `place` is `<city>` or `<district>, <city>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPlace {
    pub place: String,
    /// ISO-3166 alpha-2; `None` when the dataset row carried none.
    pub country_code: Option<String>,
}

/// Offline index of every place in the dataset.
pub struct PlaceIndex {
    entries: Vec<Entry>,
    names: String,
    /// CSR offsets into `cell_items`, one entry longer than the cell count.
    cell_starts: Vec<u32>,
    /// Entry indices grouped by grid cell.
    cell_items: Vec<u32>,
    lat_cells: usize,
    lon_cells: usize,
}

impl PlaceIndex {
    /// Parses dataset text (the derived rows, header line included).
    ///
    /// Rows that are not exactly [`COLUMNS`] wide, or that carry an empty name or
    /// unparsable/non-finite coordinates, are skipped with a warning; a dataset
    /// without the expected header or without a valid row yields `None`.
    fn parse(text: &str) -> Option<PlaceIndex> {
        let mut lines = text.lines();
        match lines.next() {
            Some(header) if header.starts_with(HEADER_PREFIX) => {}
            _ => return None,
        }
        let mut builder = Builder::sized_for(text.len());
        for (index, line) in lines.enumerate() {
            builder.push(line, index + 2);
        }
        builder.finish()
    }

    /// Number of indexed places.
    fn place_count(&self) -> usize {
        self.entries.len()
    }

    /// Resolves coordinates into a place label.
    ///
    /// The nearest place within [`MAX_DISTANCE_KM`] wins. A `PPLX` district is
    /// prefixed to the most populous parent city within
    /// [`MAX_PARENT_DISTANCE_KM`] that lies in the same country, so a district
    /// near a border is never labelled with a foreign city.
    pub fn resolve(&self, latitude: f64, longitude: f64) -> Option<ResolvedPlace> {
        if !latitude.is_finite()
            || !longitude.is_finite()
            || !(-90.0..=90.0).contains(&latitude)
            || !(-180.0..=180.0).contains(&longitude)
        {
            return None;
        }

        let nearest = self.nearest_within(latitude, longitude, MAX_DISTANCE_KM)?;
        let country_code = nearest.country_code.map(str::to_string);
        if !nearest.is_district() {
            return Some(ResolvedPlace {
                place: nearest.name.to_string(),
                country_code,
            });
        }

        match nearest.country_code.and_then(|country| {
            self.most_populous_parent_within(latitude, longitude, MAX_PARENT_DISTANCE_KM, country)
        }) {
            Some(parent) => Some(ResolvedPlace {
                place: format!("{}, {}", nearest.name, parent.name),
                country_code,
            }),
            None => Some(ResolvedPlace {
                place: nearest.name.to_string(),
                country_code,
            }),
        }
    }

    /// Nearest place within `max_distance_km` (exact haversine distance).
    ///
    /// Equal distances are resolved deterministically by the higher population,
    /// then by name, so the result does not depend on dataset order.
    fn nearest_within(
        &self,
        latitude: f64,
        longitude: f64,
        max_distance_km: f64,
    ) -> Option<Hit<'_>> {
        self.best_candidate(
            latitude,
            longitude,
            max_distance_km,
            &|_| true,
            &|candidate, candidate_distance, current, current_distance| {
                candidate_distance
                    .total_cmp(&current_distance)
                    .then_with(|| current.population.cmp(&candidate.population))
                    .then_with(|| self.name(candidate).cmp(self.name(current)))
            },
        )
    }

    /// Most populous parent city within `max_distance_km`, restricted to `country_code`.
    ///
    /// Districts (`PPLX`) are displayed as `<district>, <parent>`, and the parent
    /// is expected to be the metro area rather than the closest village — hence
    /// population first, distance as tie-breaker (and name last, for determinism).
    fn most_populous_parent_within(
        &self,
        latitude: f64,
        longitude: f64,
        max_distance_km: f64,
        country_code: &str,
    ) -> Option<Hit<'_>> {
        self.best_candidate(
            latitude,
            longitude,
            max_distance_km,
            &|entry| {
                entry.kind == PlaceKind::Parent && self.country_code(entry) == Some(country_code)
            },
            &|candidate, candidate_distance, current, current_distance| {
                current
                    .population
                    .cmp(&candidate.population)
                    .then_with(|| candidate_distance.total_cmp(&current_distance))
                    .then_with(|| self.name(candidate).cmp(self.name(current)))
            },
        )
    }

    /// Scans the candidates the bounding box can reach and keeps the best one.
    ///
    /// `compare` orders a candidate against the current best and reports
    /// [`Ordering::Less`] when the candidate should replace it; the two callers
    /// differ only in their acceptance filter and in whether distance or
    /// population comes first.
    fn best_candidate(
        &self,
        latitude: f64,
        longitude: f64,
        max_distance_km: f64,
        accept: &dyn Fn(&Entry) -> bool,
        compare: &dyn Fn(&Entry, f64, &Entry, f64) -> std::cmp::Ordering,
    ) -> Option<Hit<'_>> {
        let mut best: Option<(&Entry, f64)> = None;
        self.for_each_candidate(
            latitude,
            longitude,
            max_distance_km,
            &mut |entry, distance| {
                if distance > max_distance_km || !accept(entry) {
                    return;
                }
                let is_better = match best {
                    None => true,
                    Some((current, current_distance)) => {
                        compare(entry, distance, current, current_distance).is_lt()
                    }
                };
                if is_better {
                    best = Some((entry, distance));
                }
            },
        );
        best.map(|(entry, _)| self.hit(entry))
    }

    /// Visits every entry whose grid cell overlaps the bounding box of
    /// `max_distance_km` around the point. The box is a strict superset of the
    /// radius, so callers filter by distance.
    fn for_each_candidate<'a>(
        &'a self,
        latitude: f64,
        longitude: f64,
        max_distance_km: f64,
        visit: &mut impl FnMut(&'a Entry, f64),
    ) {
        let lat_span = max_distance_km / KM_PER_DEG_LAT + BOX_SLACK_DEG;
        let min_lat = (latitude - lat_span).max(-90.0);
        let max_lat = (latitude + lat_span).min(90.0);

        // A degree of longitude covers fewer kilometers the further the point is
        // from the equator, so the box's extremal latitude bounds the span.
        let box_lat = min_lat.abs().max(max_lat.abs()).to_radians();
        let cos_lat = box_lat.cos().abs().max(1e-6);
        let lon_span =
            (max_distance_km / (KM_PER_DEG_LAT * cos_lat) + BOX_SLACK_DEG / cos_lat).min(180.0);

        let (windows, window_count) = self.longitude_cells(longitude, lon_span);
        for lat_cell in self.lat_cell(min_lat)..=self.lat_cell(max_lat) {
            let row = lat_cell * self.lon_cells;
            for (lon_from, lon_to) in windows.iter().take(window_count) {
                for cell in row + lon_from..=row + lon_to {
                    let start = self.cell_starts[cell] as usize;
                    let end = self.cell_starts[cell + 1] as usize;
                    for &index in &self.cell_items[start..end] {
                        let entry = &self.entries[index as usize];
                        let distance =
                            haversine_km(latitude, longitude, entry.lat as f64, entry.lon as f64);
                        visit(entry, distance);
                    }
                }
            }
        }
    }

    /// Longitude cell ranges covering `longitude ± lon_span`; two ranges when the
    /// box crosses the antimeridian, clipped at ±180°.
    fn longitude_cells(&self, longitude: f64, lon_span: f64) -> ([(usize, usize); 2], usize) {
        let west = longitude - lon_span;
        let east = longitude + lon_span;
        let last = self.lon_cells - 1;
        if lon_span >= 180.0 {
            ([(0, last), (0, 0)], 1)
        } else if west < -180.0 {
            (
                [
                    (self.lon_cell(west + 360.0), last),
                    (0, self.lon_cell(east)),
                ],
                2,
            )
        } else if east > 180.0 {
            (
                [
                    (self.lon_cell(west), last),
                    (0, self.lon_cell(east - 360.0)),
                ],
                2,
            )
        } else {
            ([(self.lon_cell(west), self.lon_cell(east)), (0, 0)], 1)
        }
    }

    fn lat_cell(&self, latitude: f64) -> usize {
        (((latitude + 90.0) / CELL_DEG).floor() as i64).clamp(0, self.lat_cells as i64 - 1) as usize
    }

    fn lon_cell(&self, longitude: f64) -> usize {
        (((longitude + 180.0) / CELL_DEG).floor() as i64).clamp(0, self.lon_cells as i64 - 1)
            as usize
    }

    fn name(&self, entry: &Entry) -> &str {
        let start = entry.name_start as usize;
        &self.names[start..start + entry.name_len as usize]
    }

    fn country_code<'a>(&self, entry: &'a Entry) -> Option<&'a str> {
        let bytes = &entry.country;
        let len = if bytes[0] == 0 {
            0
        } else if bytes[1] == 0 {
            1
        } else {
            2
        };
        if len == 0 {
            return None;
        }
        std::str::from_utf8(&bytes[..len]).ok()
    }

    fn hit<'a>(&'a self, entry: &'a Entry) -> Hit<'a> {
        Hit {
            name: self.name(entry),
            country_code: self.country_code(entry),
            kind: entry.kind,
        }
    }

    /// Groups all entries into their grid cell (counting sort over cells).
    fn build(entries: Vec<Entry>, names: String) -> PlaceIndex {
        let lat_cells = (180.0 / CELL_DEG).ceil() as usize;
        let lon_cells = (360.0 / CELL_DEG).ceil() as usize;
        let cell_count = lat_cells * lon_cells;
        let mut index = PlaceIndex {
            entries,
            names,
            cell_starts: vec![0; cell_count + 1],
            cell_items: Vec::new(),
            lat_cells,
            lon_cells,
        };
        index.cell_items = vec![0; index.entries.len()];

        let mut cell_of = Vec::with_capacity(index.entries.len());
        for entry in &index.entries {
            let cell =
                index.lat_cell(entry.lat as f64) * lon_cells + index.lon_cell(entry.lon as f64);
            index.cell_starts[cell + 1] += 1;
            cell_of.push(cell);
        }
        for cell in 0..cell_count {
            index.cell_starts[cell + 1] += index.cell_starts[cell];
        }

        let mut cursor = index.cell_starts.clone();
        for (entry_index, &cell) in cell_of.iter().enumerate() {
            index.cell_items[cursor[cell] as usize] = entry_index as u32;
            cursor[cell] += 1;
        }

        index
    }
}

/// Accumulates parsed rows; kept separate from [`PlaceIndex`] so `parse` stays pure.
struct Builder {
    entries: Vec<Entry>,
    names: String,
}

impl Builder {
    fn sized_for(bytes: usize) -> Self {
        let rows = (bytes / BYTES_PER_ROW_ESTIMATE).max(1);
        Builder {
            entries: Vec::with_capacity(rows),
            names: String::with_capacity(rows * NAME_BYTES_PER_ROW_ESTIMATE),
        }
    }

    /// Parses one dataset row; malformed rows are skipped with a warning.
    fn push(&mut self, line: &str, line_number: usize) {
        if line.trim().is_empty() {
            return;
        }
        let (columns, column_count) = split_row(line);
        if column_count != COLUMNS {
            warn!(
                "skipping place dataset line {}: expected {} cols, got {}",
                line_number, COLUMNS, column_count
            );
            return;
        }

        let name = columns[NAME_COLUMN].trim();
        if name.is_empty() {
            warn!("skipping place dataset line {}: empty name", line_number);
            return;
        }
        if name.len() > u16::MAX as usize {
            warn!(
                "skipping place dataset line {}: name longer than {} bytes",
                line_number,
                u16::MAX
            );
            return;
        }

        let Ok(latitude) = columns[LATITUDE_COLUMN].parse::<f64>() else {
            warn!(
                "skipping place dataset line {}: invalid latitude '{}'",
                line_number, columns[LATITUDE_COLUMN]
            );
            return;
        };
        if !latitude.is_finite() || !(-90.0..=90.0).contains(&latitude) {
            warn!(
                "skipping place dataset line {}: out-of-range latitude '{}'",
                line_number, columns[LATITUDE_COLUMN]
            );
            return;
        }
        let Ok(longitude) = columns[LONGITUDE_COLUMN].parse::<f64>() else {
            warn!(
                "skipping place dataset line {}: invalid longitude '{}'",
                line_number, columns[LONGITUDE_COLUMN]
            );
            return;
        };
        if !longitude.is_finite() || !(-180.0..=180.0).contains(&longitude) {
            warn!(
                "skipping place dataset line {}: out-of-range longitude '{}'",
                line_number, columns[LONGITUDE_COLUMN]
            );
            return;
        }

        let mut country = [0u8; 2];
        for (slot, byte) in country
            .iter_mut()
            .zip(columns[COUNTRY_COLUMN].trim().bytes())
        {
            *slot = byte.to_ascii_uppercase();
        }
        let population = columns[POPULATION_COLUMN]
            .trim()
            .parse::<i64>()
            .map(|value| value.max(0) as u32)
            .unwrap_or(0);

        let name_start = self.names.len() as u32;
        self.names.push_str(name);
        self.entries.push(Entry {
            name_start,
            name_len: name.len() as u16,
            country,
            kind: kind_of(
                columns[FEATURE_CLASS_COLUMN].trim(),
                columns[FEATURE_CODE_COLUMN].trim(),
            ),
            lat: latitude as f32,
            lon: longitude as f32,
            population,
        });
    }

    fn finish(mut self) -> Option<PlaceIndex> {
        if self.entries.is_empty() {
            return None;
        }
        self.entries.shrink_to_fit();
        self.names.shrink_to_fit();
        Some(PlaceIndex::build(self.entries, self.names))
    }
}

/// Splits the leading `COLUMNS` columns and reports how many the row has in
/// total; avoids one heap allocation per dataset row.
fn split_row(line: &str) -> ([&str; COLUMNS], usize) {
    let mut columns = [""; COLUMNS];
    let mut count = 0;
    for column in line.split('\t') {
        if count < COLUMNS {
            columns[count] = column;
        }
        count += 1;
    }
    (columns, count)
}

/// Reduces GeoNames feature class/code to the kinds resolution distinguishes.
fn kind_of(feature_class: &str, feature_code: &str) -> PlaceKind {
    if feature_class != "P" {
        return PlaceKind::Other;
    }
    match feature_code {
        "PPL" | "PPLA" | "PPLA2" | "PPLA3" | "PPLA4" | "PPLC" | "PPLG" | "PPLS" => {
            PlaceKind::Parent
        }
        "PPLX" => PlaceKind::District,
        _ => PlaceKind::Other,
    }
}

fn haversine_km(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let dlat = (lat2 - lat1).to_radians();
    let dlon = (lon2 - lon1).to_radians();
    let a = (dlat / 2.0).sin().powi(2)
        + lat1.to_radians().cos() * lat2.to_radians().cos() * (dlon / 2.0).sin().powi(2);
    let c = 2.0 * a.clamp(0.0, 1.0).sqrt().asin();
    EARTH_RADIUS_KM * c
}

/// The embedded dataset. `build.rs` guarantees it exists before compilation.
const DATASET: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/geodata.txt"));

/// The process-wide index, parsed from [`DATASET`] on first use.
///
/// A `OnceLock` rather than a `LazyLock` keeps a failed parse retryable: the
/// cell is only filled with a successfully built index.
static INDEX: OnceLock<PlaceIndex> = OnceLock::new();

/// The shared index, or `None` when the embedded dataset held no valid row.
pub fn embedded_index() -> Option<&'static PlaceIndex> {
    if let Some(index) = INDEX.get() {
        return Some(index);
    }
    let index = match PlaceIndex::parse(DATASET) {
        Some(index) => {
            log::info!(
                "loaded {} places for offline geo resolution",
                index.place_count()
            );
            index
        }
        None => {
            warn!("embedded place dataset holds no valid row; geo resolution disabled");
            return None;
        }
    };
    // A concurrent caller may have won the race; either index is equivalent.
    let _ = INDEX.set(index);
    INDEX.get()
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: &str = "#turbo-pix-places-v1\tname\tlatitude\tlongitude\tfeature_class\tfeature_code\tcountry_code\tpopulation";

    fn row(
        name: &str,
        latitude: &str,
        longitude: &str,
        feature_class: &str,
        feature_code: &str,
        country: &str,
        population: &str,
    ) -> String {
        [
            name,
            latitude,
            longitude,
            feature_class,
            feature_code,
            country,
            population,
        ]
        .join("\t")
    }

    fn city(
        name: &str,
        latitude: &str,
        longitude: &str,
        country: &str,
        population: &str,
    ) -> String {
        row(name, latitude, longitude, "P", "PPL", country, population)
    }

    fn district(name: &str, latitude: &str, longitude: &str, country: &str) -> String {
        row(name, latitude, longitude, "P", "PPLX", country, "5000")
    }

    fn index_of(rows: &[String]) -> PlaceIndex {
        let text = format!("{HEADER}\n{}", rows.join("\n"));
        PlaceIndex::parse(&text).expect("test dataset should contain valid rows")
    }

    fn place(index: &PlaceIndex, latitude: f64, longitude: f64) -> Option<ResolvedPlace> {
        index.resolve(latitude, longitude)
    }

    #[test]
    fn a_city_within_the_radius_resolves_to_its_name_and_country() {
        // GIVEN one city ~22 km north of a query point
        let index = index_of(&[city("Kiel", "54.4", "10.0", "DE", "246601")]);

        // WHEN resolving the point
        let resolved = place(&index, 54.2, 10.0);

        // THEN the city and its country are returned
        assert_eq!(
            resolved,
            Some(ResolvedPlace {
                place: "Kiel".to_string(),
                country_code: Some("DE".to_string()),
            })
        );
    }

    #[test]
    fn a_city_beyond_the_radius_resolves_to_nothing() {
        // GIVEN one city ~33 km north of a query point
        let index = index_of(&[city("Kiel", "54.5", "10.0", "DE", "246601")]);

        // WHEN resolving the point
        let resolved = place(&index, 54.2, 10.0);

        // THEN the photo keeps its coordinates only
        assert_eq!(resolved, None);
    }

    #[test]
    fn a_district_is_prefixed_with_the_most_populous_parent_not_the_nearest() {
        // GIVEN a district between a small near town and a large city ~12 km away
        let index = index_of(&[
            district("Volksdorf", "53.6510", "10.1660", "DE"),
            city("Ahrensburg", "53.6740", "10.2260", "DE", "33561"),
            city("Hamburg", "53.5510", "9.9930", "DE", "1845229"),
        ]);

        // WHEN resolving a point in the district
        let resolved = place(&index, 53.6510, 10.1660);

        // THEN the label is "district, most populous city within 30 km"
        assert_eq!(
            resolved.map(|value| value.place),
            Some("Volksdorf, Hamburg".to_string())
        );
    }

    #[test]
    fn a_parent_in_another_country_is_never_chosen() {
        // GIVEN a German district, a much larger French city ~5 km away and a
        // smaller German city ~20 km away
        let index = index_of(&[
            district("Grenzdorf", "47.5900", "7.6000", "DE"),
            city("Mulhouse", "47.6200", "7.6200", "FR", "108312"),
            city("Lörrach", "47.6100", "7.6600", "DE", "49389"),
        ]);

        // WHEN resolving the district
        let resolved = place(&index, 47.5900, 7.6000);

        // THEN the same-country parent is used
        assert_eq!(
            resolved.map(|value| value.place),
            Some("Grenzdorf, Lörrach".to_string())
        );
    }

    #[test]
    fn a_district_without_a_same_country_parent_keeps_its_own_name() {
        // GIVEN a German district whose only large neighbour lies in France
        let index = index_of(&[
            district("Grenzdorf", "47.5900", "7.6000", "DE"),
            city("Mulhouse", "47.6200", "7.6200", "FR", "108312"),
        ]);

        // WHEN resolving the district
        let resolved = place(&index, 47.5900, 7.6000);

        // THEN the district is shown alone, with its own country
        assert_eq!(
            resolved,
            Some(ResolvedPlace {
                place: "Grenzdorf".to_string(),
                country_code: Some("DE".to_string()),
            })
        );
    }

    #[test]
    fn a_plain_city_is_not_prefixed_with_a_parent() {
        // GIVEN a city and a larger city next to it
        let index = index_of(&[
            city("Ahrensburg", "53.6740", "10.2260", "DE", "33561"),
            city("Hamburg", "53.5510", "9.9930", "DE", "1845229"),
        ]);

        // WHEN resolving the smaller city
        let resolved = place(&index, 53.6740, 10.2260);

        // THEN only the city itself is shown
        assert_eq!(
            resolved.map(|value| value.place),
            Some("Ahrensburg".to_string())
        );
    }

    #[test]
    fn a_place_of_the_wrong_feature_class_is_never_a_parent() {
        // GIVEN a land-feature row (class T) next to a district
        let index = index_of(&[
            district("Volksdorf", "53.6510", "10.1660", "DE"),
            row("Wald", "53.6520", "10.1670", "T", "PPL", "DE", "999999"),
        ]);

        // WHEN resolving the district
        let resolved = place(&index, 53.6510, 10.1660);

        // THEN the non-place row is not used as a parent
        assert_eq!(
            resolved.map(|value| value.place),
            Some("Volksdorf".to_string())
        );
    }

    #[test]
    fn coordinates_across_the_antimeridian_find_the_place() {
        // GIVEN a place just east of the date line
        let index = index_of(&[city("Dateline", "0.0", "179.95", "FJ", "600")]);

        // WHEN resolving a point ~11 km west of the date line
        let resolved = place(&index, 0.0, -179.95);

        // THEN the place is found across the wrap
        assert_eq!(
            resolved.map(|value| value.place),
            Some("Dateline".to_string())
        );
    }

    #[test]
    fn non_finite_coordinates_resolve_to_nothing() {
        let index = index_of(&[city("Kiel", "54.4", "10.0", "DE", "246601")]);

        assert_eq!(place(&index, f64::NAN, 10.0), None);
        assert_eq!(place(&index, 54.2, f64::INFINITY), None);
    }

    #[test]
    fn malformed_rows_are_skipped_and_the_rest_still_loads() {
        // GIVEN a dataset with one short row and one good row
        let text = format!(
            "{HEADER}\nnot-a-row\n{}",
            city("Kiel", "54.4", "10.0", "DE", "246601")
        );

        let index = PlaceIndex::parse(&text).expect("the good row must load");

        assert_eq!(index.place_count(), 1);
        assert_eq!(
            place(&index, 54.2, 10.0).map(|value| value.place),
            Some("Kiel".to_string())
        );
    }

    #[test]
    fn a_dataset_without_the_expected_header_is_refused() {
        let text = format!("{}\n", city("Kiel", "54.4", "10.0", "DE", "246601"));

        assert!(PlaceIndex::parse(&text).is_none());
    }

    #[test]
    fn the_embedded_dataset_resolves_a_real_coordinate() {
        // GIVEN the dataset this binary embeds
        let index = embedded_index().expect("the embedded dataset must parse");

        // WHEN resolving the centre of Berlin
        let resolved = index
            .resolve(52.5200, 13.4050)
            .expect("Berlin must resolve");

        // THEN the label carries the city name and its country
        assert!(
            resolved.place.contains("Berlin"),
            "unexpected label: {}",
            resolved.place
        );
        assert_eq!(resolved.country_code.as_deref(), Some("DE"));
    }

    #[test]
    fn the_embedded_dataset_covers_the_expected_row_count() {
        let index = embedded_index().expect("the embedded dataset must parse");

        assert!(
            index.place_count() > 390_000,
            "the embedded dataset is incomplete: {} rows",
            index.place_count()
        );
    }

    #[test]
    fn non_ascii_names_round_trip() {
        // GIVEN a dataset row with German and Danish names
        let index = index_of(&[
            city("München", "48.1372", "11.5756", "DE", "1260391"),
            city("København", "55.6761", "12.5683", "DK", "644431"),
        ]);

        assert_eq!(
            place(&index, 48.1372, 11.5756).map(|value| value.place),
            Some("München".to_string())
        );
        assert_eq!(
            place(&index, 55.6761, 12.5683).map(|value| value.place),
            Some("København".to_string())
        );
    }
}

// Pure metadata-edit policy for the metadata modal: it decides what the PATCH
// payload may contain and which form states must be refused. It knows nothing
// about i18n, the DOM or the API — refusals are codes the component maps to
// translated messages — so it can be tested under `node --test`.
//
// Two rules shape the payload:
//  * A field the form was opened with and nobody touched is omitted. The
//    backend rewrites every date carrier whenever it sees `taken_at`, and its
//    location writer for any position pair, so restating an untouched value
//    would rewrite bytes the user never looked at.
//  * The position travels as a unit: a half pair is refused rather than sent
//    as a request the backend answers with a refusal the user cannot act on.
//
// `baseline` describes the form's opening state: the `takenAt`/`latitude`/
// `longitude` strings installed on open plus whether the photo actually had
// each value (`hadTakenAt`, `hadLatitude`, `hadLongitude`) — the flags decide
// the clear refusal, the strings decide whether a value changed.

const trimmed = (value) => String(value ?? '').trim();

/**
 * The number a coordinate input stands for, or `null` when it holds nothing
 * parseable. Both sides of the "did it change" comparison go through this, so
 * "52.50" and "52.5" are the same position, not an edit.
 */
function coordinateValue(input) {
  const parsed = parseFloat(String(input ?? '').trim());
  return Number.isNaN(parsed) ? null : parsed;
}

/**
 * Build the metadata PATCH payload from the form's current values.
 *
 * Returns `{ updates }` on success — possibly empty when nothing changed — or
 * `{ refusal }` with one of:
 *  * `clear_unsupported` — a previously-set field was emptied; the backend
 *    cannot clear these fields, so the save must fail honestly. Checked first,
 *    so a cleared field is never reported as a pairing/range problem.
 *  * `gps_pair` — only one of latitude/longitude was provided.
 *  * `gps_range` — a coordinate is not a finite number in range
 *    ([-90, 90] / [-180, 180]).
 *  * `invalid_date` — a non-empty date input that does not parse; never
 *    silently saved as "unchanged".
 */
export function buildMetadataEdit({ takenAt, latitude, longitude, baseline } = {}) {
  const base = baseline ?? {};
  const takenAtInput = trimmed(takenAt);
  const latitudeInput = trimmed(latitude);
  const longitudeInput = trimmed(longitude);

  // `baseline` describes the form's opening state: the `takenAt`/`latitude`/
  // `longitude` strings installed on open plus whether the photo actually had
  // each value (`hadTakenAt`, `hadLatitude`, `hadLongitude`) — the flags decide
  // the clear refusal, the strings decide whether a value changed.
  const hadTakenAt = base.hadTakenAt === true;
  const hadLatitude = base.hadLatitude === true;
  const hadLongitude = base.hadLongitude === true;

  if (
    (hadTakenAt && takenAtInput === '') ||
    (hadLatitude && latitudeInput === '') ||
    (hadLongitude && longitudeInput === '')
  ) {
    return { refusal: 'clear_unsupported' };
  }

  const updates = {};

  // Only an edited date goes into the payload. The comparison is on the
  // instant, so the input's own spelling (with or without seconds) never
  // counts as an edit. An unparseable value is not emitted: the input is a
  // `datetime-local` control, which yields either a valid instant or nothing.
  if (takenAtInput !== '') {
    const takenAtMs = new Date(takenAtInput).getTime();
    if (!Number.isFinite(takenAtMs)) {
      return { refusal: 'invalid_date' };
    }
    const baselineMs = trimmed(base.takenAt) ? new Date(trimmed(base.takenAt)).getTime() : null;
    if (takenAtMs !== baselineMs) {
      updates.taken_at = new Date(takenAtMs).toISOString();
    }
  }

  const hasLatitude = latitudeInput !== '';
  const hasLongitude = longitudeInput !== '';
  if (hasLatitude !== hasLongitude) {
    return { refusal: 'gps_pair' };
  }

  if (hasLatitude && hasLongitude) {
    const latitudeValue = parseFloat(latitudeInput);
    const longitudeValue = parseFloat(longitudeInput);
    if (
      !Number.isFinite(latitudeValue) ||
      !Number.isFinite(longitudeValue) ||
      latitudeValue < -90 ||
      latitudeValue > 90 ||
      longitudeValue < -180 ||
      longitudeValue > 180
    ) {
      return { refusal: 'gps_range' };
    }

    // The pair is all-or-nothing and goes in only when it differs from the
    // value the form opened with (see the untouched-value rule above).
    const positionChanged =
      latitudeValue !== coordinateValue(base.latitude) ||
      longitudeValue !== coordinateValue(base.longitude);
    if (positionChanged) {
      updates.latitude = latitudeValue;
      updates.longitude = longitudeValue;
    }
  }

  return { updates };
}

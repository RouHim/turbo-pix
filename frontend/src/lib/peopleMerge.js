/**
 * Pure decisions of the People merge dialog.
 *
 * The limit protects against "select all visible" + merge: one accidental
 * click must not collapse a whole library into one person.
 */
export const MAX_MERGE_PERSONS = 21;

/** Merge needs a keeper and at least one source, and stays under the limit. */
export const canMergeSelection = (count) => count >= 2 && count <= MAX_MERGE_PERSONS;

/**
 * Default name for the merged person: the keeper's name wins; an unnamed
 * keeper borrows the only named source, so a merge never discards the only
 * human-readable name for `Person {id}`. Ambiguity (several distinct names)
 * leaves the field empty instead of guessing.
 */
export const mergeNamePrefill = (selected, keeperId) => {
  const keeperName = selected.find((item) => String(item.id) === String(keeperId))?.name?.trim();
  if (keeperName) return keeperName;
  const sourceNames = selected
    .filter((item) => String(item.id) !== String(keeperId))
    .map((item) => item.name?.trim())
    .filter(Boolean);
  return sourceNames.length === 1 ? sourceNames[0] : '';
};

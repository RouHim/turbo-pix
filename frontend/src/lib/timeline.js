// Pure view-model for the desktop year-rail + month-strip navigator.
// No Svelte imports: testable with node --test.

/**
 * @param {Array<{ year: number, month: number, count: number }>} density
 * @returns {Array<{ year: number, total: number, months: Array<{ month: number, count: number }> }>}
 */
export const buildYearAggregates = (density) => {
  const byYear = new Map();
  for (const { year, month, count } of density ?? []) {
    if (!byYear.has(year)) {
      byYear.set(
        year,
        Array.from({ length: 12 }, (_, i) => ({ month: i + 1, count: 0 }))
      );
    }
    const slot = byYear.get(year)[month - 1];
    if (slot) slot.count += count;
  }
  return [...byYear.entries()]
    .sort(([a], [b]) => b - a)
    .map(([year, months]) => ({
      year,
      total: months.reduce((sum, m) => sum + m.count, 0),
      months,
    }));
};

/**
 * @param {Array<{ year: number, total: number, months: Array<{ month: number, count: number }> }>} aggregates
 * @param {number | null} year
 */
export const getYearAggregate = (aggregates, year) =>
  aggregates.find((a) => a.year === year) ?? null;

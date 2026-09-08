<script>
  import { locale } from 'svelte-i18n';
  import { t } from '../lib/i18n.js';
  import { api } from '../lib/api.js';
  import { addToast } from '../lib/state.svelte.js';
  import { route, pushState } from '../lib/router.svelte.js';
  import { APP_CONSTANTS } from '../lib/constants.js';
  import { buildYearAggregates, getYearAggregate } from '../lib/timeline.js';
  import Icon from './Icon.svelte';

  const activeLocale = $derived($locale || 'en');

  let data = $state(null);
  let currentFilter = $state(null);
  let selectedYear = $state(null);
  let yearSelectEl = $state(null);
  let monthSelectEl = $state(null);
  let initError = $state(false);

  const aggregates = $derived(buildYearAggregates(data?.density ?? []));

  // The mobile dropdowns below are intentionally untouched and still iterate
  // `years`; derive it from aggregates (same descending order) so they keep
  // working until a later task rewrites them.
  const years = $derived(aggregates.map((a) => a.year));

  const selectedAggregate = $derived(
    selectedYear === null ? null : getYearAggregate(aggregates, selectedYear)
  );

  const labelText = $derived.by(() => {
    if (!currentFilter) {
      return $t('ui.all_dates', { locale: activeLocale, default: 'All Dates' });
    }
    if (!currentFilter.month) {
      return String(currentFilter.year);
    }
    return monthYearLabel(currentFilter.year, currentFilter.month);
  });

  const monthYearLabel = (year, month) => {
    const monthKey = APP_CONSTANTS.MONTH_KEYS[month - 1];
    const monthName = $t(`ui.months.${monthKey}`, {
      locale: activeLocale,
      default: `${monthKey.charAt(0).toUpperCase()}${monthKey.slice(1)}`,
    });
    return `${monthName} ${year}`;
  };

  $effect(() => {
    fetchTimelineData();
  });

  const fetchTimelineData = async () => {
    try {
      data = await api.request('/api/photos/timeline');
    } catch (error) {
      console.error('Failed to initialize timeline:', error);
      addToast(
        $t('notifications.error', { default: 'Error' }),
        $t('errors.timeline_load_failed', { default: 'Failed to load timeline data' }),
        'error',
        4000
      );
      initError = true;
    }
  };

  const pushFilter = () => {
    const year = currentFilter?.year ?? null;
    const month = currentFilter?.month ?? null;
    pushState({ year, month: year ? month : null });
  };

  const selectYear = (year) => {
    if (selectedYear === year && (currentFilter?.month ?? null) === null) {
      currentFilter = null;
      selectedYear = null;
    } else if (selectedYear === year) {
      currentFilter = { year, month: null };
    } else {
      currentFilter = { year, month: null };
      selectedYear = year;
    }
    if (selectedYear !== null && currentFilter !== null && currentFilter.year !== selectedYear) {
      selectedYear = currentFilter.year;
    }
    if (currentFilter === null) selectedYear = null;
    pushFilter();
  };

  const selectMonth = (month, count) => {
    if (count === 0 || selectedYear === null) return;
    if (currentFilter?.month === month) {
      currentFilter = { year: selectedYear, month: null };
    } else {
      currentFilter = { year: selectedYear, month };
    }
    pushFilter();
  };

  const resetFilter = () => {
    currentFilter = null;
    selectedYear = null;
    if (yearSelectEl) yearSelectEl.value = '';
    if (monthSelectEl) monthSelectEl.value = '';
    pushFilter();
  };

  const handleDropdownChange = () => {
    const year = yearSelectEl?.value;
    let month = monthSelectEl?.value;
    if (!year) {
      month = null;
      if (monthSelectEl) monthSelectEl.value = '';
    }
    if (!year && !month) {
      currentFilter = null;
      selectedYear = null;
    } else {
      const parsedYear = year ? parseInt(year, 10) : null;
      currentFilter = {
        year: parsedYear,
        month: month ? parseInt(month, 10) : null,
      };
      selectedYear = parsedYear;
    }
    pushFilter();
  };

  const restoreFilterFromRoute = (year, month) => {
    if (!year && !month) {
      if (currentFilter) {
        currentFilter = null;
        selectedYear = null;
        if (yearSelectEl) yearSelectEl.value = '';
        if (monthSelectEl) monthSelectEl.value = '';
      }
    } else if (year) {
      currentFilter = { year, month: month || null };
      selectedYear = year;
      if (yearSelectEl) yearSelectEl.value = String(year);
      if (monthSelectEl) monthSelectEl.value = month ? String(month) : '';
    }
  };

  // Restore filter from route state (URL restore / popstate).
  // Reads route BEFORE any guard: an early return that reads nothing empties
  // the effect's dependency set and permanently unsubscribes it.
  $effect(() => {
    const year = route.year;
    const month = route.month;
    restoreFilterFromRoute(year, month);
  });
</script>

{#if !initError}
  {#if !data}
    <div class="timeline-container">
      <div class="timeline-slider">
        <div class="timeline-skeleton" aria-hidden="true"></div>
        <div class="timeline-label">{labelText}</div>
      </div>
    </div>
  {:else if aggregates.length === 0}
    <!-- Empty library: nothing to filter, render nothing -->
  {:else}
    <div class="timeline-container">
      <!-- Desktop: Year rail + month strip -->
      <div class="timeline-rail desktop-only">
        <button
          type="button"
          class="timeline-reset"
          title={$t('ui.clear_timeline_filter', { default: 'Clear timeline filter' })}
          aria-label={$t('ui.clear_timeline_filter', { default: 'Clear timeline filter' })}
          onclick={resetFilter}
        >
          <Icon name="x" width={14} height={14} />
        </button>
        <div
          class="timeline-year-rail"
          role="group"
          aria-label={$t('ui.timeline_years_label', { default: 'Years' })}
        >
          {#each aggregates as agg (agg.year)}
            <button
              type="button"
              class="timeline-year"
              class:active={selectedYear === agg.year}
              aria-pressed={selectedYear === agg.year}
              aria-label={`${agg.year}, ${$t('ui.photos_count', { values: { count: agg.total }, default: '{count} photos' })}`}
              onclick={() => selectYear(agg.year)}
            >
              <span class="timeline-year-label">{agg.year}</span>
              <span class="timeline-year-count">{agg.total}</span>
            </button>
          {/each}
        </div>
        {#if selectedAggregate}
          {@const agg = selectedAggregate}
          <div
            class="timeline-month-strip"
            role="group"
            aria-label={$t('ui.timeline_months_label', { default: 'Months' })}
          >
            {#each agg.months as slot (slot.month)}
              {@const monthKey = APP_CONSTANTS.MONTH_KEYS[slot.month - 1]}
              {@const monthName = $t(`ui.months.${monthKey}`, {
                locale: activeLocale,
                default: `${monthKey.charAt(0).toUpperCase()}${monthKey.slice(1)}`,
              })}
              <button
                type="button"
                class="timeline-month"
                class:active={currentFilter?.month === slot.month}
                class:empty={slot.count === 0}
                disabled={slot.count === 0}
                aria-pressed={currentFilter?.month === slot.month}
                aria-label={slot.count === 0
                  ? `${monthName} ${agg.year}, ${$t('ui.timeline_no_photos_month', { default: 'No photos' })}`
                  : `${monthName} ${agg.year}, ${$t('ui.photos_count', { values: { count: slot.count }, default: '{count} photos' })}`}
                onclick={() => selectMonth(slot.month, slot.count)}
              >
                <span class="timeline-month-label">{monthName}</span>
                <span class="timeline-month-count">{slot.count}</span>
              </button>
            {/each}
          </div>
        {/if}
        <div class="timeline-label" class:filtered={currentFilter !== null}>{labelText}</div>
      </div>

      <!-- Mobile: Dropdowns -->
      <div class="timeline-dropdowns mobile-only">
        <select
          id="timeline-year-select"
          class="timeline-year-select"
          bind:this={yearSelectEl}
          aria-label={$t('ui.year_select', { default: 'Year' })}
          onchange={handleDropdownChange}
        >
          <option value="">{$t('ui.all_years', { default: 'All Years' })}</option>
          {#each years as year (year)}
            <option value={year}>{year}</option>
          {/each}
        </select>
        <select
          id="timeline-month-select"
          class="timeline-month-select"
          bind:this={monthSelectEl}
          aria-label={$t('ui.month_select', { default: 'Month' })}
          disabled={!currentFilter?.year}
          onchange={handleDropdownChange}
        >
          <option value="">{$t('ui.all_months', { default: 'All Months' })}</option>
          {#each APP_CONSTANTS.MONTH_KEYS as monthKey, i (i)}
            <option value={i + 1}
              >{$t(`ui.months.${monthKey}`, {
                default: monthKey.charAt(0).toUpperCase() + monthKey.slice(1),
              })}</option
            >
          {/each}
        </select>
        <button
          type="button"
          class="timeline-reset"
          title={$t('ui.clear_timeline_filter', { default: 'Clear timeline filter' })}
          aria-label={$t('ui.clear_timeline_filter', { default: 'Clear timeline filter' })}
          onclick={resetFilter}
        >
          <Icon name="x" width={14} height={14} />
        </button>
      </div>
    </div>
  {/if}
{/if}

<style>
  .timeline-container {
    margin: var(--space-6) 0;
    padding: var(--space-5) var(--space-6);
    background: var(--surface-color);
    border-radius: var(--radius-md);
    border: 1px solid var(--divider-color);
    box-shadow: var(--shadow-light);
  }

  .timeline-label {
    flex-shrink: 0;
    min-width: 104px;
    text-align: center;
    padding: var(--space-1) var(--space-3);
    border-radius: var(--radius-full);
    font-size: var(--font-sm);
    font-weight: var(--font-medium);
    color: var(--text-secondary);
    transition:
      background-color var(--transition-fast),
      color var(--transition-fast);
  }

  .timeline-label.filtered {
    background: color-mix(in oklch, var(--primary-color) 12%, transparent);
    color: var(--primary-dark);
  }

  .timeline-reset {
    flex-shrink: 0;
    width: var(--space-8);
    height: var(--space-8);
    border-radius: var(--radius-full);
    border: 1px solid var(--divider-color);
    background: transparent;
    color: var(--text-secondary);
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    transition:
      border-color var(--transition-fast),
      color var(--transition-fast),
      background-color var(--transition-fast);
  }

  .timeline-reset:hover {
    border-color: var(--primary-color);
    color: var(--primary-color);
    background: color-mix(in oklch, var(--primary-color) 10%, transparent);
  }

  .timeline-reset:active {
    transform: translateY(1px);
  }

  .timeline-reset:focus-visible {
    outline: none;
    border-color: var(--primary-color);
    box-shadow:
      0 0 0 2px var(--surface-color),
      0 0 0 4px var(--primary-color);
  }
  .timeline-rail {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-start;
    gap: var(--space-4);
  }
  .timeline-year-rail {
    flex: 1;
    display: flex;
    gap: var(--space-2);
    min-width: 0;
    overflow-x: auto;
    flex-wrap: nowrap;
    padding-block: var(--space-1);
  }
  .timeline-year {
    flex: 0 0 auto;
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--divider-color);
    border-radius: var(--radius-full);
    background: transparent;
    color: var(--text-primary);
    cursor: pointer;
    white-space: nowrap;
  }
  .timeline-year:hover {
    border-color: var(--primary-color);
  }
  .timeline-year.active {
    background: color-mix(in oklch, var(--primary-color) 12%, transparent);
    border-color: var(--primary-color);
    color: var(--primary-dark);
  }
  .timeline-year:focus-visible,
  .timeline-month:focus-visible,
  .timeline-reset:focus-visible {
    outline: none;
    box-shadow:
      0 0 0 2px var(--surface-color),
      0 0 0 4px var(--primary-color);
  }
  .timeline-year-label {
    font-size: var(--font-sm);
    font-weight: var(--font-medium);
  }
  .timeline-year-count,
  .timeline-month-count {
    font-size: var(--font-xs);
    color: var(--text-secondary);
  }
  .timeline-month-strip {
    display: grid;
    flex: 1 1 100%;
    order: 1;
    grid-template-columns: repeat(6, minmax(0, 1fr));
    gap: var(--space-2);
    margin-top: var(--space-3);
  }
  .timeline-month {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--divider-color);
    border-radius: var(--radius-md);
    background: transparent;
    color: var(--text-primary);
    cursor: pointer;
    min-width: 0;
  }
  .timeline-month:hover:not(:disabled) {
    border-color: var(--primary-color);
  }
  .timeline-month.active {
    background: color-mix(in oklch, var(--primary-color) 12%, transparent);
    border-color: var(--primary-color);
  }
  .timeline-month.empty {
    opacity: 0.55;
    cursor: not-allowed;
  }
  .timeline-rail :global(svg) {
    width: 14px;
    height: 14px;
  }

  .timeline-skeleton {
    flex: 1;
    height: 40px;
    border-radius: var(--radius-lg);
    background: var(--background-secondary);
    border: 1px solid var(--divider-color);
  }

  /* Mobile Timeline Dropdowns */
  .timeline-dropdowns {
    display: none;
    gap: var(--space-3);
    align-items: center;
  }

  .timeline-year-select,
  .timeline-month-select {
    flex: 1;
    min-width: 0;
    padding: var(--space-2) var(--space-3);
    border: 2px solid var(--divider-color);
    border-radius: var(--radius-md);
    background: var(--surface-color);
    color: var(--text-primary);
    font-size: var(--font-base);
    cursor: pointer;
    transition: var(--transition-fast);
  }

  .timeline-year-select:focus-visible,
  .timeline-month-select:focus-visible {
    outline: none;
    border-color: var(--primary-color);
    box-shadow:
      0 0 0 2px var(--surface-color),
      0 0 0 4px var(--primary-color);
  }

  .timeline-year-select:disabled,
  .timeline-month-select:disabled {
    opacity: 0.55;
    cursor: not-allowed;
  }

  .desktop-only {
    display: flex;
  }

  .mobile-only {
    display: none;
  }

  @media (width <= 768px) {
    .desktop-only {
      display: none;
    }

    .mobile-only {
      display: flex;
    }

    .timeline-container {
      padding: var(--space-3) var(--space-4);
    }
  }

  @media (width <= 480px) {
    .timeline-dropdowns {
      gap: var(--space-2);
      padding: var(--space-3) var(--space-2);
    }

    .timeline-year-select,
    .timeline-month-select {
      min-width: 0;
    }

    .timeline-reset {
      width: 36px;
      height: 36px;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .timeline-year,
    .timeline-month,
    .timeline-label,
    .timeline-label.filtered,
    .timeline-reset {
      animation: none;
      transition: none;
    }
  }
</style>

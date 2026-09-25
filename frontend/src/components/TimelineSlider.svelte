<script>
  import { untrack } from 'svelte';
  import { locale } from 'svelte-i18n';
  import { t } from '../lib/i18n.js';
  import { api } from '../lib/api.js';
  import { addToast } from '../lib/state.svelte.js';
  import { route, pushState, replaceState } from '../lib/router.svelte.js';
  import { APP_CONSTANTS } from '../lib/constants.js';
  import { buildTimelineModel, formatPeriodName, formatSelectionLabel } from '../lib/timeline.js';
  import {
    filterEquals,
    filterFromSelection,
    normalizeDateFilter,
    selectionFromFilter,
  } from '../lib/timelineRoute.js';
  import TimelineSelector from './TimelineSelector.svelte';
  import Icon from './Icon.svelte';

  const activeLocale = $derived($locale || 'en');

  let data = $state(null);
  let yearSelectEl = $state(null);
  let monthSelectEl = $state(null);
  let initError = $state(false);

  const model = $derived(buildTimelineModel(data?.density ?? []));
  const filter = $derived(
    normalizeDateFilter({
      year: route.year,
      month: route.month,
      to_year: route.to_year,
      to_month: route.to_month,
    })
  );
  const selection = $derived(selectionFromFilter(filter, model));

  const labelText = $derived.by(() => {
    const monthName = (month) => {
      const monthKey = APP_CONSTANTS.MONTH_KEYS[month - 1];
      return $t(`ui.months.${monthKey}`, { locale: activeLocale, default: monthKey });
    };
    return formatSelectionLabel(selection, {
      allDates: $t('ui.all_dates', { locale: activeLocale, default: 'All Dates' }),
      monthName,
      periodName: (index) => formatPeriodName(index, monthName),
      rangeTemplate: (start, end) =>
        $t('ui.timeline_range_label', { values: { start, end }, default: '{start} – {end}' }),
      decadeLabel: (year) =>
        $t('ui.timeline_decade_label', {
          locale: activeLocale,
          values: { start: String(year) },
          default: '{start}s',
        }),
    });
  });

  let resetNonce = $state(0);
  let liveTimer = null;
  let pendingLive = null;

  const LIVE_COMMIT_MS = 100;

  // Live scrubbing must not spam history or the grid: the overlay follows the
  // pointer immediately, the route (and therefore the grid) trails by <=100 ms.
  // A live write of `null` is a real instruction — the pinch hands an
  // interrupted brush back to no selection — so the trailing write applies
  // whatever was queued; the timer is only ever armed by such a write.
  const handleChange = (next, { commit = true } = {}) => {
    if (commit) {
      if (liveTimer !== null) {
        clearTimeout(liveTimer);
        liveTimer = null;
        pendingLive = null;
      }
      pushState(filterFromSelection(next));
      return;
    }
    pendingLive = next;
    liveTimer ??= setTimeout(() => {
      liveTimer = null;
      const pending = pendingLive;
      pendingLive = null;
      replaceState(filterFromSelection(pending));
    }, LIVE_COMMIT_MS);
  };

  const clearFilter = () => {
    resetNonce += 1;
    pushState({ year: null, month: null, to_year: null, to_month: null });
  };

  // Mobile dropdowns: the plan shape is a single period, so using either
  // dropdown collapses an active desktop range to its start period.
  const handleDropdownChange = () => {
    const year = yearSelectEl?.value ? parseInt(yearSelectEl.value, 10) : null;
    const month = year !== null && monthSelectEl?.value ? parseInt(monthSelectEl.value, 10) : null;
    pushState({ year, month, to_year: null, to_month: null });
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

  // Rewrite the route to the clamped, canonical filter once the density has
  // actually loaded — an in-flight or failed fetch must never be mistaken for
  // "the data is gone" and wipe a restored filter. Reads every dependency
  // BEFORE any guard (AGENTS.md #2).
  $effect(() => {
    const loaded = data !== null;
    const current = filter;
    const canonical = filterFromSelection(selection);
    untrack(() => {
      if (!loaded || filterEquals(current, canonical)) return;
      replaceState(canonical);
    });
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
  {:else if model.length === 0}
    <!-- Empty library: nothing to filter, render nothing -->
  {:else}
    <div class="timeline-container">
      <!-- Desktop: zoomable overview -->
      <div class="timeline-rail desktop-only">
        <div class="timeline-header">
          <div class="timeline-label" class:filtered={selection !== null}>{labelText}</div>
          <button
            type="button"
            class="timeline-reset"
            title={$t('ui.clear_timeline_filter', { default: 'Clear timeline filter' })}
            aria-label={$t('ui.clear_timeline_filter', { default: 'Clear timeline filter' })}
            onclick={clearFilter}
          >
            <Icon name="x" width={14} height={14} />
          </button>
        </div>
        <TimelineSelector {model} {selection} onchange={handleChange} {resetNonce} />
      </div>

      <!-- Mobile: Dropdowns (unchanged behaviour; a desktop range collapses to
           its start period when one of these is used) -->
      <div class="timeline-dropdowns mobile-only">
        <select
          id="timeline-year-select"
          class="timeline-year-select"
          bind:this={yearSelectEl}
          aria-label={$t('ui.year_select', { default: 'Year' })}
          value={filter.year === null ? '' : String(filter.year)}
          onchange={handleDropdownChange}
        >
          <option value="">{$t('ui.all_years', { default: 'All Years' })}</option>
          {#each model.years as year (year)}
            <option value={String(year)}>{year}</option>
          {/each}
        </select>
        <select
          id="timeline-month-select"
          class="timeline-month-select"
          bind:this={monthSelectEl}
          aria-label={$t('ui.month_select', { default: 'Month' })}
          disabled={filter.year === null}
          value={filter.month === null ? '' : String(filter.month)}
          onchange={handleDropdownChange}
        >
          <option value="">{$t('ui.all_months', { default: 'All Months' })}</option>
          {#each APP_CONSTANTS.MONTH_KEYS as monthKey, i (i)}
            <option value={String(i + 1)}
              >{$t(`ui.months.${monthKey}`, { default: monthKey })}</option
            >
          {/each}
        </select>
        <button
          type="button"
          class="timeline-reset"
          title={$t('ui.clear_timeline_filter', { default: 'Clear timeline filter' })}
          aria-label={$t('ui.clear_timeline_filter', { default: 'Clear timeline filter' })}
          onclick={clearFilter}
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

  /* Single-class selector on purpose: the mobile block's `.desktop-only
     { display: none }` must win on source order, which a compound
     `.timeline-rail.desktop-only` (higher specificity) would defeat. */
  .timeline-rail {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    width: 100%;
  }

  .timeline-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
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
    .timeline-label,
    .timeline-label.filtered,
    .timeline-reset {
      animation: none;
      transition: none;
    }
  }
</style>

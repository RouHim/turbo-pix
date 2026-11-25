<script>
  import { untrack } from 'svelte';
  import { locale } from 'svelte-i18n';
  import { t } from '../lib/i18n.js';
  import { loadTimeline } from '../lib/api.js';
  import { addToast, appState, setTimelineCollapsed } from '../lib/state.svelte.js';
  import { route, pushState, replaceState } from '../lib/router.svelte.js';
  import { buildTimelineModel, formatPeriodName, formatSelectionLabel } from '../lib/timeline.js';
  import { formatMonthName } from '../lib/datetime.js';
  import {
    filterEquals,
    filterFromSelection,
    normalizeDateFilter,
    selectionFromFilter,
  } from '../lib/timelineRoute.js';
  import TimelineSelector from './TimelineSelector.svelte';
  import Icon from './Icon.svelte';

  const activeLocale = $derived($locale || 'en');
  const MONTHS = Array.from({ length: 12 }, (_, i) => i + 1);

  let data = $state(null);
  let yearSelectEl = $state(null);
  let monthSelectEl = $state(null);
  let initError = $state(false);
  let timelineRequest = 0;
  // Desktop-only fold; the mobile dropdown row has no toggle of its own.
  let detailsEl = $state(null);
  let collapseToggleEl = $state(null);

  const model = $derived(buildTimelineModel(data?.density ?? []));
  // The timeline earns its 210px only once the density can say something:
  // fewer than three distinct years is a grid, not a graph.
  const showTimeline = $derived(model.years.length > 2);
  const filter = $derived(
    normalizeDateFilter({
      year: route.year,
      month: route.month,
      to_year: route.to_year,
      to_month: route.to_month,
    })
  );
  const selection = $derived(selectionFromFilter(filter, model));
  // The route filter is the source of truth for the clear control: without one
  // the button has nothing to do and must not burn a history entry.
  const hasFilter = $derived(
    filter.year !== null ||
      filter.month !== null ||
      filter.to_year !== null ||
      filter.to_month !== null
  );

  // A decade filter names a year the library has no bucket in (1960 while the
  // oldest bucket is March 1962); without it the select would fall back to
  // "All Years" while the grid stays filtered.
  const dropdownYears = $derived(
    filter.year === null || model.years.includes(filter.year)
      ? model.years
      : [filter.year, ...model.years].sort((a, b) => b - a)
  );

  const labelText = $derived.by(() => {
    const monthName = (month) => formatMonthName(month, activeLocale);
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

  const collapseLabel = $derived(
    appState.timelineCollapsed
      ? $t('ui.expand_timeline', { default: 'Expand timeline' })
      : $t('ui.collapse_timeline', { default: 'Collapse timeline' })
  );

  // The state is purely presentational: it never touches the route filter.
  // Focus inside the selector is captured before `hidden` removes it from the
  // tab order, so collapsing from the keyboard cannot strand focus on <body>.
  const toggleCollapsed = () => {
    const focusWasInside = detailsEl?.contains(document.activeElement) ?? false;
    const next = !appState.timelineCollapsed;
    setTimelineCollapsed(next);
    if (next && focusWasInside) collapseToggleEl?.focus();
  };

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
      const nextFilter = filterFromSelection(next);
      // FR-010: re-activating the period that is already the filter — or ending
      // a gesture exactly where it started — is not a state change, and a
      // duplicate history entry would make Back appear to do nothing. `filter`
      // is the normalized route filter, so a non-canonical restored URL is still
      // left to the canonicalising effect below instead of being rewritten here.
      if (filterEquals(nextFilter, filter)) return;
      pushState(nextFilter);
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

  const clearFilter = (event) => {
    if (!hasFilter) return;
    const activated = event?.currentTarget;
    const hadFocus = activated && document.activeElement === activated;
    resetNonce += 1;
    pushState({ year: null, month: null, to_year: null, to_month: null });
    if (!hadFocus) return;
    // The activation disables the control it came from; keep focus in the
    // header instead of dropping it onto <body>.
    if (collapseToggleEl?.getClientRects().length) collapseToggleEl.focus();
    else yearSelectEl?.focus();
  };

  // Mobile dropdowns: the plan shape is a single period, so using either
  // dropdown collapses an active desktop range to its start period.
  const handleDropdownChange = () => {
    const year = yearSelectEl?.value ? parseInt(yearSelectEl.value, 10) : null;
    const month = year !== null && monthSelectEl?.value ? parseInt(monthSelectEl.value, 10) : null;
    pushState({ year, month, to_year: null, to_month: null });
  };

  $effect(() => {
    void fetchTimelineData();
    // A saved date edit moves a photo between buckets; the density is a
    // server-side aggregate, so the whole graph has to be re-read. Registering
    // is not a reactive read (the dependency set stays empty), and the teardown
    // keeps a remount from stacking listeners. The loader joins the date
    // scrubber's identical refetch into one request.
    const handleReload = () => void fetchTimelineData(true);
    window.addEventListener('photosReloadRequested', handleReload);
    return () => window.removeEventListener('photosReloadRequested', handleReload);
  });

  const fetchTimelineData = async (force = false) => {
    // A forced reload starts a fresh read while the initial one may still be
    // in flight; only the newest answer may replace the rendered graph.
    const request = ++timelineRequest;
    try {
      const next = await loadTimeline(force);
      if (request !== timelineRequest) return;
      data = next;
      initError = false;
    } catch (error) {
      if (request !== timelineRequest) return;
      console.error('Failed to initialize timeline:', error);
      addToast(
        $t('notifications.error', { default: 'Error' }),
        $t('errors.timeline_load_failed', { default: 'Failed to load timeline data' }),
        'error',
        4000
      );
      // Only the initial load is fatal: with nothing rendered there is no
      // timeline to keep. A refetch that fails after a save must leave an
      // already-rendered graph in place; the next success clears the flag.
      if (data === null) initError = true;
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
      </div>
    </div>
  {:else if !showTimeline}
    <!-- Too few distinct years to describe density: the grid leads. -->
  {:else}
    <div class="timeline-container">
      <!-- Desktop: zoomable overview -->
      <div class="timeline-rail desktop-only">
        <div class="timeline-header">
          <button
            type="button"
            class="timeline-fold"
            data-testid="timeline-collapse-toggle"
            bind:this={collapseToggleEl}
            title={collapseLabel}
            aria-label={collapseLabel}
            aria-describedby={selection !== null ? 'timeline-filter-label' : undefined}
            aria-expanded={!appState.timelineCollapsed}
            aria-controls="timeline-details"
            onclick={toggleCollapsed}
          >
            <span class="timeline-heading">
              <span class="timeline-title">{$t('ui.timeline_title', { default: 'Timeline' })}</span>
              {#if selection !== null}
                <span id="timeline-filter-label" class="timeline-label">{labelText}</span>
              {/if}
            </span>
            <span
              class="timeline-chevron"
              class:expanded={!appState.timelineCollapsed}
              aria-hidden="true"
            >
              <Icon name="chevron-down" width={14} height={14} />
            </span>
          </button>
          <button
            type="button"
            class="timeline-reset"
            disabled={!hasFilter}
            title={$t('ui.timeline_reset', { default: 'Reset timeline' })}
            aria-label={$t('ui.timeline_reset', { default: 'Reset timeline' })}
            onclick={clearFilter}
          >
            <Icon name="rotate-ccw" width={16} height={16} />
          </button>
        </div>
        <div id="timeline-details" bind:this={detailsEl} hidden={appState.timelineCollapsed}>
          <TimelineSelector {model} {selection} onchange={handleChange} {resetNonce} />
        </div>
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
          {#each dropdownYears as year (year)}
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
          {#each MONTHS as month (month)}
            <option value={String(month)}>{formatMonthName(month, activeLocale)}</option>
          {/each}
        </select>
        <button
          type="button"
          class="timeline-reset"
          disabled={!hasFilter}
          title={$t('ui.timeline_reset', { default: 'Reset timeline' })}
          aria-label={$t('ui.timeline_reset', { default: 'Reset timeline' })}
          onclick={clearFilter}
        >
          <Icon name="rotate-ccw" width={16} height={16} />
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

  /* The section name anchors the fold row; it inherits the toggle's color so
     hover and focus stay one interaction. The active filter sits beside it as
     a state pill, never in place of the name. */
  .timeline-heading {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }

  .timeline-title {
    flex-shrink: 0;
    font-size: var(--font-sm);
    font-weight: var(--font-medium);
    white-space: nowrap;
  }

  .timeline-label {
    flex-shrink: 0;
    text-align: center;
    padding: var(--space-1) var(--space-3);
    border-radius: var(--radius-full);
    background: color-mix(in oklch, var(--primary-color) 12%, transparent);
    color: var(--primary-ink);
    font-size: var(--font-sm);
    font-weight: var(--font-medium);
    white-space: nowrap;
  }

  .timeline-fold {
    flex: 1;
    min-width: 0;
    height: var(--space-8);
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    padding: 0;
    border: none;
    border-radius: var(--radius-md);
    background: transparent;
    color: var(--text-secondary);
    font: inherit;
    cursor: pointer;
    transition:
      background-color var(--transition-fast),
      color var(--transition-fast);
  }

  .timeline-fold:hover {
    background: color-mix(in oklch, var(--primary-color) 6%, transparent);
    color: var(--primary-color);
  }

  .timeline-fold:focus-visible {
    outline: none;
    color: var(--primary-color);
    box-shadow:
      0 0 0 2px var(--surface-color),
      0 0 0 4px var(--primary-color);
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

  .timeline-reset:hover:not(:disabled) {
    border-color: var(--primary-color);
    color: var(--primary-color);
    background: color-mix(in oklch, var(--primary-color) 10%, transparent);
  }

  .timeline-reset:active:not(:disabled) {
    transform: translateY(1px);
  }

  .timeline-reset:focus-visible {
    outline: none;
    border-color: var(--primary-color);
    box-shadow:
      0 0 0 2px var(--surface-color),
      0 0 0 4px var(--primary-color);
  }

  .timeline-reset:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .timeline-chevron {
    display: flex;
    margin-left: auto;
    transition: transform var(--transition-fast);
  }

  .timeline-chevron.expanded {
    transform: rotate(180deg);
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
    gap: var(--space-2);
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

    .timeline-reset {
      width: var(--button-size);
      height: var(--button-size);
    }

    .timeline-year-select,
    .timeline-month-select {
      min-height: var(--button-size);
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
      width: var(--button-size);
      height: var(--button-size);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .timeline-fold,
    .timeline-reset,
    .timeline-chevron {
      animation: none;
      transition: none;
    }
  }
</style>

<script>
  import { onDestroy } from 'svelte';
  import { locale } from 'svelte-i18n';
  import { t } from '../lib/i18n.js';
  import { api } from '../lib/api.js';
  import { addToast } from '../lib/state.svelte.js';
  import { route, pushState } from '../lib/router.svelte.js';
  import { APP_CONSTANTS } from '../lib/constants.js';
  import Icon from '../lib/Icon.svelte';

  const activeLocale = $derived($locale || 'en');

  let data = $state(null);
  let currentFilter = $state(null);
  let debounceTimer = null;
  let selectedIndex = $state(null);
  let sliderValue = $state(0);
  let ribbonEl = $state(null);
  let yearSelectEl = $state(null);
  let monthSelectEl = $state(null);
  let initError = $state(false);
  let hoveredIndex = $state(null);
  let tooltipX = $state(0);
  let tooltipY = $state(0);
  // Set while a desktop slider drag is being debounced; the route-restore
  // $effect must not wipe the in-progress filter before the URL push fires.
  let dragInProgress = false;

  const positions = $derived(
    data?.density?.map((d) => ({
      year: d.year,
      month: d.month,
      count: d.count,
    })) ?? []
  );

  const years = $derived(
    data?.density ? [...new Set(data.density.map((d) => d.year))].sort((a, b) => b - a) : []
  );

  const maxSlider = $derived(Math.max(0, positions.length - 1));

  const maxCount = $derived(Math.max(1, ...positions.map((p) => p.count)));

  // Year landmarks: start bucket index per year; dense rule when > 6 years.
  // Ordered ascending (positions are oldest-first) so ticks read left to right
  // under the ribbon; the first tick is left-aligned to avoid clipping.
  const yearTicks = $derived.by(() => {
    const ticks = [...years]
      .sort((a, b) => a - b)
      .map((year) => ({
        year,
        startIndex: positions.findIndex((p) => p.year === year),
      }));
    if (ticks.length <= 6) return ticks;
    return ticks.filter((_, i) => i === 0 || i === ticks.length - 1 || i % 2 === 0);
  });

  function monthYearLabel(year, month) {
    const monthKey = APP_CONSTANTS.MONTH_KEYS[month - 1];
    const monthName = $t(`ui.months.${monthKey}`, {
      locale: activeLocale,
      default: monthKey.charAt(0).toUpperCase() + monthKey.slice(1),
    });
    return `${monthName} ${year}`;
  }

  const labelText = $derived.by(() => {
    if (!currentFilter) {
      return $t('ui.all_dates', { locale: activeLocale, default: 'All Dates' });
    }
    if (!currentFilter.month) {
      return String(currentFilter.year);
    }
    return monthYearLabel(currentFilter.year, currentFilter.month);
  });

  $effect(() => {
    fetchTimelineData();
  });

  async function fetchTimelineData() {
    try {
      data = await api.request('/api/photos/timeline');
      if (data?.density?.length > 0) {
        sliderValue = data.density.length - 1;
      }
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
  }

  function applyFilter(updateUrl = true) {
    if (updateUrl) {
      const year = currentFilter?.year ?? null;
      const month = currentFilter?.month ?? null;
      // Never push month without year
      pushState({ year, month: year ? month : null });
    }
  }

  function resetFilter() {
    currentFilter = null;
    selectedIndex = null;
    if (positions.length > 0) {
      sliderValue = positions.length - 1;
    }
    if (yearSelectEl) yearSelectEl.value = '';
    if (monthSelectEl) monthSelectEl.value = '';
    applyFilter();
  }

  function handleSliderInput(e) {
    const index = parseInt(e.target.value);
    if (index >= positions.length - 1) {
      currentFilter = null;
      selectedIndex = null;
    } else {
      const pos = positions[index];
      currentFilter = { year: pos.year, month: pos.month };
      selectedIndex = index;
    }
    // The route-restore $effect reads currentFilter as a dependency, so it
    // re-runs on every drag tick — with route.year still null it would reset
    // the filter and snap the thumb back before the debounced push fires.
    dragInProgress = true;
    clearTimeout(debounceTimer);
    debounceTimer = setTimeout(() => {
      dragInProgress = false;
      applyFilter();
    }, 300);
  }

  function handleDropdownChange() {
    const year = yearSelectEl?.value;
    let month = monthSelectEl?.value;
    if (!year) {
      // A month without a year is not a valid filter — clear the stale selection.
      month = null;
      if (monthSelectEl) monthSelectEl.value = '';
    }
    if (!year && !month) {
      currentFilter = null;
    } else {
      currentFilter = {
        year: year ? parseInt(year) : null,
        month: month ? parseInt(month) : null,
      };
    }
    applyFilter();
  }

  function handleTrackHover(e) {
    if (!ribbonEl || positions.length === 0) return;
    const rect = ribbonEl.getBoundingClientRect();
    if (rect.width === 0) return;
    const barWidth = rect.width / positions.length;
    let index = Math.floor((e.clientX - rect.left) / barWidth);
    if (index < 0 || index >= positions.length) index = -1;
    hoveredIndex = index >= 0 ? index : null;
    // Clamp so the tooltip never overflows the viewport (it is centered via
    // translateX(-50%) and offset 60px above the cursor).
    tooltipX = Math.min(Math.max(e.clientX, 110), window.innerWidth - 110);
    tooltipY = Math.max(e.clientY - 60, 8);
  }

  function handleTrackLeave() {
    hoveredIndex = null;
  }

  /**
   * Applies a route-driven filter (URL restore / popstate). Must run before
   * the drag guard in the effect below — see the comment there.
   */
  function restoreFilterFromRoute(year, month) {
    if (!year && !month) {
      // Reset to no filter (only if we have a current filter)
      if (currentFilter) {
        currentFilter = null;
        selectedIndex = null;
        if (positions.length > 0) {
          sliderValue = positions.length - 1;
        }
        if (yearSelectEl) yearSelectEl.value = '';
        if (monthSelectEl) monthSelectEl.value = '';
      }
    } else if (year) {
      const matchIndex = positions.findIndex((p) => p.year === year && p.month === month);
      currentFilter = { year, month: month || null };
      selectedIndex = matchIndex >= 0 ? matchIndex : null;
      if (matchIndex >= 0) {
        sliderValue = matchIndex;
      }
      if (yearSelectEl && year) yearSelectEl.value = String(year);
      if (monthSelectEl) monthSelectEl.value = month ? String(month) : '';
    }
  }

  // Restore filter from route state (URL restore / popstate)
  $effect(() => {
    // Read route BEFORE the drag guard: an early return that reads nothing
    // empties the effect's dependency set and permanently unsubscribes it
    // (Svelte 5 replaces deps with what this run read).
    const year = route.year;
    const month = route.month;
    if (dragInProgress) return;
    restoreFilterFromRoute(year, month);
  });

  onDestroy(() => {
    if (debounceTimer) clearTimeout(debounceTimer);
  });
</script>

{#if !initError}
  <div class="timeline-container">
    {#if !data}
      <div class="timeline-slider desktop-only">
        <div class="timeline-skeleton" aria-hidden="true"></div>
        <div class="timeline-label">{labelText}</div>
      </div>
    {:else if positions.length === 0}
      <!-- Empty library: nothing to filter, render nothing -->
    {:else}
      <!-- Desktop: Slider -->
      <div class="timeline-slider desktop-only">
        <button
          type="button"
          class="timeline-reset"
          title={$t('ui.clear_timeline_filter', { default: 'Clear timeline filter' })}
          aria-label={$t('ui.clear_timeline_filter', { default: 'Clear timeline filter' })}
          onclick={resetFilter}
        >
          <Icon name="x" width={14} height={14} />
        </button>
        <div class="timeline-track-stack">
          <!-- The track itself is presentational: hover only drives the tooltip, the range input below is the keyboard-accessible control -->
          <div
            class="timeline-track"
            role="presentation"
            onmousemove={handleTrackHover}
            onmouseleave={handleTrackLeave}
          >
            <div class="timeline-groove" aria-hidden="true">
              <div class="timeline-ribbon" bind:this={ribbonEl}>
                {#each positions as pos, i (i)}
                  <div
                    class="timeline-bar"
                    class:selected={selectedIndex === i}
                    class:hovered={hoveredIndex === i}
                    style="--bar-ratio: {pos.count / maxCount}; --bar-opacity: {0.35 +
                      0.55 * (pos.count / maxCount)}; animation-delay: {Math.min(i * 8, 300)}ms"
                  ></div>
                {/each}
              </div>
            </div>
            <input
              type="range"
              class="timeline-input"
              min="0"
              max={maxSlider}
              value={sliderValue}
              aria-label={$t('ui.timeline_slider_aria', { default: 'Timeline filter' })}
              aria-valuetext={labelText}
              oninput={handleSliderInput}
              ondblclick={resetFilter}
            />
            {#if hoveredIndex !== null}
              {@const pos = positions[hoveredIndex]}
              <div class="timeline-tooltip" style="left: {tooltipX}px; top: {tooltipY}px">
                <div class="timeline-tooltip-date">{monthYearLabel(pos.year, pos.month)}</div>
                <div class="timeline-tooltip-count">
                  {$t('ui.photos_count', {
                    values: { count: pos.count },
                    default: '{count} photos',
                  })}
                </div>
              </div>
            {/if}
          </div>
          <div class="timeline-ticks" aria-hidden="true">
            {#each yearTicks as tick, i (tick.year)}
              <span
                class="timeline-year-tick"
                class:first={i === 0}
                style="left: {(tick.startIndex / positions.length) * 100}%">{tick.year}</span
              >
            {/each}
          </div>
        </div>
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
    {/if}
  </div>
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

  .timeline-slider {
    display: flex;
    align-items: center;
    gap: var(--space-5);
  }

  .timeline-track-stack {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
  }

  .timeline-track {
    position: relative;
    height: 40px;
    display: flex;
    align-items: center;
  }

  .timeline-groove {
    position: absolute;
    inset: 4px 0;
    display: flex;
    align-items: center;
    background: var(--background-secondary);
    border: 1px solid var(--divider-color);
    border-radius: var(--radius-lg);
    padding: 2px 4px;
  }

  .timeline-ribbon {
    display: flex;
    align-items: flex-end;
    gap: 2px;
    width: 100%;
    height: 100%;
  }

  .timeline-bar {
    flex: 1 1 0;
    min-width: 0;
    height: calc(6px + var(--bar-ratio) * 20px);
    border-radius: var(--radius-full);
    background: var(--primary-color);
    opacity: var(--bar-opacity);
    transform-origin: bottom;
    animation: timeline-bar-grow 0.4s var(--ease-spring) backwards;
    transition: opacity var(--transition-fast);
  }

  .timeline-bar.selected {
    opacity: 1;
  }

  .timeline-bar.hovered {
    opacity: 0.9;
  }

  @keyframes timeline-bar-grow {
    from {
      transform: scaleY(0);
    }
    to {
      transform: scaleY(1);
    }
  }

  .timeline-input {
    position: absolute;
    top: 50%;
    left: 0;
    width: 100%;
    transform: translateY(-50%);
    z-index: 2;
    -webkit-appearance: none;
    appearance: none;
    background: transparent;
    cursor: grab;
    height: 40px;
    margin: 0;
    border-radius: var(--radius-lg);
  }

  .timeline-input:active {
    cursor: grabbing;
  }

  .timeline-input:focus-visible {
    outline: none;
    box-shadow:
      0 0 0 2px var(--surface-color),
      0 0 0 4px var(--primary-color);
  }

  .timeline-input::-webkit-slider-thumb {
    -webkit-appearance: none;
    appearance: none;
    width: 24px;
    height: 24px;
    border-radius: var(--radius-full);
    background: var(--primary-color);
    border: 2px solid var(--surface-color);
    cursor: grab;
    box-shadow: 0 1px 4px oklch(0% 0 0deg / 20%);
    transition: transform var(--transition-fast);
  }

  .timeline-input::-webkit-slider-thumb:hover {
    transform: scale(1.1);
    box-shadow: 0 2px 8px oklch(0% 0 0deg / 30%);
  }

  .timeline-input:active::-webkit-slider-thumb {
    cursor: grabbing;
    transform: scale(1.05);
  }

  .timeline-input::-moz-range-thumb {
    width: 24px;
    height: 24px;
    border-radius: var(--radius-full);
    background: var(--primary-color);
    border: 2px solid var(--surface-color);
    cursor: grab;
    box-shadow: 0 1px 4px oklch(0% 0 0deg / 20%);
    transition: transform var(--transition-fast);
  }

  .timeline-input::-moz-range-thumb:hover {
    transform: scale(1.1);
    box-shadow: 0 2px 8px oklch(0% 0 0deg / 30%);
  }

  .timeline-input:active::-moz-range-thumb {
    cursor: grabbing;
    transform: scale(1.05);
  }

  .timeline-ticks {
    position: relative;
    height: 16px;
    overflow: hidden;
    pointer-events: none;
  }

  .timeline-year-tick {
    position: absolute;
    top: 0;
    transform: translateX(-50%);
    font-size: var(--font-xs);
    line-height: 16px;
    color: var(--text-secondary);
    white-space: nowrap;
    user-select: none;
  }

  .timeline-year-tick.first {
    transform: none;
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

  .timeline-tooltip {
    position: fixed;
    transform: translateX(-50%);
    background: var(--surface-elevated);
    border: 1px solid var(--divider-color);
    border-radius: var(--radius-md);
    padding: var(--space-2) var(--space-3);
    box-shadow: var(--shadow-heavy);
    pointer-events: none;
    z-index: var(--z-tooltip);
    animation: timeline-tooltip-in 0.15s ease-out;
  }

  @keyframes timeline-tooltip-in {
    from {
      opacity: 0;
    }
    to {
      opacity: 1;
    }
  }

  .timeline-tooltip-date {
    font-size: var(--font-md);
    font-weight: var(--font-semibold);
    color: var(--text-primary);
    margin-bottom: var(--space-1);
  }

  .timeline-tooltip-count {
    font-size: var(--font-sm);
    color: var(--text-secondary);
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
    .timeline-bar,
    .timeline-label,
    .timeline-tooltip,
    .timeline-input::-webkit-slider-thumb,
    .timeline-reset {
      animation: none;
      transition: none;
    }
    /* NOTE: keep the -moz thumb in its own rule — Chromium drops the WHOLE
       selector list when it contains an unknown pseudo-element, which would
       empty this media query (and `animation: none` never applied). */
    .timeline-input::-moz-range-thumb {
      animation: none;
      transition: none;
    }
  }
</style>

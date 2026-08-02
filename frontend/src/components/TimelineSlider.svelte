<script>
  import { onDestroy } from 'svelte';
  import { locale } from 'svelte-i18n';
  import { t } from '../lib/i18n.js';
  import { api } from '../lib/api.js';
  import { route, pushState } from '../lib/router.svelte.js';
  import { APP_CONSTANTS } from '../lib/constants.js';
  import Icon from '../lib/Icon.svelte';

  const activeLocale = $derived($locale || 'en');

  let data = $state(null);
  let currentFilter = $state(null);
  let debounceTimer = null;
  let selectedIndex = $state(null);
  let sliderValue = $state(0);
  let canvasEl = $state(null);
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
      initError = true;
    }
  }

  $effect(() => {
    if (data && canvasEl) {
      renderHeatmap();
    }
  });

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
    renderHeatmap();
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
    renderHeatmap();
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
    if (!canvasEl || positions.length === 0) return;
    const rect = canvasEl.getBoundingClientRect();
    if (rect.width === 0) return;
    const barWidth = rect.width / positions.length;
    let index = Math.floor((e.clientX - rect.left) / barWidth);
    if (index < 0 || index >= positions.length) index = -1;
    hoveredIndex = index >= 0 ? index : null;
    // Clamp so the tooltip never overflows the viewport (it is centered via
    // translateX(-50%) and offset 60px above the cursor).
    tooltipX = Math.min(Math.max(e.clientX, 110), window.innerWidth - 110);
    tooltipY = Math.max(e.clientY - 60, 8);
    renderHeatmap();
  }

  function handleTrackLeave() {
    hoveredIndex = null;
    renderHeatmap();
  }

  function getThemePrimaryColor() {
    const cs = window.getComputedStyle(document.documentElement);
    return (cs.getPropertyValue('--primary-color') || '').trim();
  }

  function withAlpha(color, alpha) {
    if (!color) return `rgba(99, 102, 241, ${alpha})`;
    if (color.startsWith('oklch')) {
      return color.endsWith(')') ? `${color.slice(0, -1)} / ${alpha})` : `${color} / ${alpha}`;
    }
    if (color.startsWith('#')) {
      const hex = color.replace('#', '');
      const full =
        hex.length === 3
          ? hex
              .split('')
              .map((c) => c + c)
              .join('')
          : hex;
      const r = parseInt(full.slice(0, 2), 16);
      const g = parseInt(full.slice(2, 4), 16);
      const b = parseInt(full.slice(4, 6), 16);
      return `rgba(${r}, ${g}, ${b}, ${alpha})`;
    }
    return `rgba(99, 102, 241, ${alpha})`;
  }

  function renderHeatmap() {
    if (!canvasEl || !data?.density) return;
    const ctx = canvasEl.getContext('2d');
    const width = canvasEl.width;
    const height = canvasEl.height;
    ctx.clearRect(0, 0, width, height);
    if (positions.length === 0) return;

    const maxCount = Math.max(...data.density.map((d) => d.count));
    const primary = getThemePrimaryColor();
    drawYearMarkers(ctx, width, height, primary);

    const barWidth = width / positions.length;
    positions.forEach((pos, index) => {
      const normalizedHeight = (pos.count / maxCount) * height;
      const x = index * barWidth;
      const y = height - normalizedHeight;
      const isSelected = selectedIndex === index;

      const opacity = 0.3 + (pos.count / maxCount) * 0.7;

      ctx.fillStyle = withAlpha(primary, opacity);
      ctx.fillRect(x, y, barWidth - 1, normalizedHeight);

      if (isSelected) {
        ctx.strokeStyle = withAlpha(primary, 1);
        ctx.lineWidth = 3;
        ctx.strokeRect(x + 1, y, barWidth - 3, normalizedHeight);
        ctx.shadowBlur = 8;
        ctx.shadowColor = withAlpha(primary, 0.6);
        ctx.fillStyle = withAlpha(primary, 0.9);
        ctx.fillRect(x, y, barWidth - 1, normalizedHeight);
        ctx.shadowBlur = 0;
      } else if (hoveredIndex === index) {
        ctx.fillStyle = withAlpha(primary, 0.85);
        ctx.fillRect(x, y, barWidth - 1, normalizedHeight);
      }
    });
  }

  function drawYearMarkers(ctx, width, height, primary) {
    if (positions.length === 0) return;
    const barWidth = width / positions.length;
    let lastYear = null;
    positions.forEach((pos, index) => {
      if (pos.year !== lastYear) {
        const x = index * barWidth;
        if (lastYear !== null) {
          ctx.strokeStyle = withAlpha(primary, 0.15);
          ctx.lineWidth = 1;
          ctx.beginPath();
          ctx.moveTo(x, 0);
          ctx.lineTo(x, height);
          ctx.stroke();
        }
        lastYear = pos.year;
      }
    });
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
        renderHeatmap();
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
      renderHeatmap();
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
    <!-- Desktop: Slider -->
    <div class="timeline-slider desktop-only">
      <button
        type="button"
        class="timeline-reset"
        title={$t('ui.clear_timeline_filter', { default: 'Clear timeline filter' })}
        onclick={resetFilter}
      >
        <Icon name="x" width={14} height={14} />
      </button>
      <!-- The track itself is presentational: hover only drives the tooltip, the range input below is the keyboard-accessible control -->
      <div
        class="timeline-track"
        role="presentation"
        onmousemove={handleTrackHover}
        onmouseleave={handleTrackLeave}
      >
        <canvas class="timeline-heatmap" width="800" height="40" bind:this={canvasEl}></canvas>
        <input
          type="range"
          class="timeline-input"
          min="0"
          max={maxSlider}
          value={sliderValue}
          aria-label={$t('ui.timeline_slider_aria', { default: 'Timeline filter' })}
          oninput={handleSliderInput}
          ondblclick={resetFilter}
        />
        {#if hoveredIndex !== null}
          {@const pos = positions[hoveredIndex]}
          <div class="timeline-tooltip" style="left: {tooltipX}px; top: {tooltipY}px">
            <div class="timeline-tooltip-date">{monthYearLabel(pos.year, pos.month)}</div>
            <div class="timeline-tooltip-count">
              {$t('ui.photos_count', { values: { count: pos.count }, default: '{count} photos' })}
            </div>
          </div>
        {/if}
      </div>
      <div class="timeline-label">{labelText}</div>
    </div>

    <!-- Mobile: Dropdowns -->
    <div class="timeline-dropdowns mobile-only">
      <select
        id="timeline-year-select"
        class="timeline-year-select"
        bind:this={yearSelectEl}
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
        onclick={resetFilter}
      >
        <Icon name="x" width={14} height={14} />
      </button>
    </div>
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
    transition: var(--transition-medium);
  }

  .timeline-container:hover {
    box-shadow: var(--shadow-medium);
  }

  .timeline-slider {
    display: flex;
    align-items: center;
    gap: var(--space-5);
  }

  .timeline-reset {
    flex-shrink: 0;
    width: var(--space-8);
    height: var(--space-8);
    border-radius: var(--radius-sm);
    border: 1.5px solid var(--divider-color);
    background: var(--background-color);
    color: var(--text-muted);
    cursor: pointer;
    transition: all 0.2s cubic-bezier(0.4, 0, 0.2, 1);
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: var(--font-lg);
    font-weight: var(--font-semibold);
    position: relative;
    overflow: hidden;
  }

  .timeline-reset::before {
    content: '';
    position: absolute;
    inset: 0;
    background: var(--primary-color);
    opacity: 0;
    transition: opacity 0.2s;
  }

  .timeline-reset:hover::before {
    opacity: 1;
  }

  .timeline-reset:hover {
    border-color: var(--primary-color);
    color: white;
    box-shadow: var(--shadow-medium);
  }

  .timeline-reset:active {
    transform: translateY(0);
  }

  .timeline-track {
    flex: 1;
    position: relative;
    height: var(--button-size-lg);
    display: flex;
    align-items: center;
  }

  .timeline-heatmap {
    position: absolute;
    top: 50%;
    transform: translateY(-50%);
    left: 0;
    width: 100%;
    height: 28px;
    pointer-events: none;
    border-radius: 14px;
    box-shadow: inset 0 2px 4px rgb(0 0 0 / 10%);
    background: linear-gradient(to bottom, rgb(0 0 0 / 5%), transparent);
  }

  .timeline-input {
    position: absolute;
    top: 50%;
    left: 0;
    width: 100%;
    transform: translateY(-50%);
    -webkit-appearance: none;
    appearance: none;
    background: transparent;
    cursor: grab;
    height: var(--button-size-lg);
  }

  .timeline-input:active {
    cursor: grabbing;
  }

  .timeline-input::-webkit-slider-thumb {
    -webkit-appearance: none;
    appearance: none;
    width: 16px;
    height: 16px;
    border-radius: 8px;
    background: var(--primary-color);
    cursor: grab;
    border: 3px solid white;
    box-shadow: 0 1px 4px oklch(0% 0 0deg / 20%);
    transition: all 0.2s cubic-bezier(0.4, 0, 0.2, 1);
  }

  .timeline-input::-webkit-slider-thumb:hover {
    transform: scale(1.15);
    box-shadow: 0 2px 8px oklch(0% 0 0deg / 30%);
  }

  .timeline-input:active::-webkit-slider-thumb {
    cursor: grabbing;
    transform: scale(1.1);
  }

  .timeline-input::-moz-range-thumb {
    width: 16px;
    height: 16px;
    border-radius: 8px;
    background: var(--primary-color);
    cursor: grab;
    border: 3px solid white;
    box-shadow: 0 1px 4px oklch(0% 0 0deg / 20%);
    transition: all 0.2s cubic-bezier(0.4, 0, 0.2, 1);
  }

  .timeline-input::-moz-range-thumb:hover {
    transform: scale(1.15);
    box-shadow: 0 2px 8px oklch(0% 0 0deg / 30%);
  }

  .timeline-input:active::-moz-range-thumb {
    cursor: grabbing;
    transform: scale(1.1);
  }

  .timeline-label {
    flex-shrink: 0;
    min-width: 130px;
    text-align: right;
    font-size: var(--font-md);
    font-weight: var(--font-semibold);
    color: var(--primary-color);
    letter-spacing: -0.01em;
  }

  .timeline-tooltip {
    position: fixed;
    transform: translateX(-50%);
    background: var(--surface-color);
    border: 1px solid var(--divider-color);
    border-radius: var(--radius-md);
    padding: var(--space-3) var(--space-4);
    box-shadow: var(--shadow-heavy);
    pointer-events: none;
    z-index: var(--z-modal);
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

  /* Mobile Timeline Dropdowns */
  .timeline-dropdowns {
    display: none;
    gap: 10px;
    align-items: center;
  }

  .timeline-year-select,
  .timeline-month-select {
    flex: 1;
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--divider-color);
    border-radius: var(--radius-sm);
    background: var(--background-color);
    color: var(--text-primary);
    font-size: var(--font-base);
    cursor: pointer;
  }

  .timeline-year-select:focus,
  .timeline-month-select:focus {
    outline: none;
    border-color: var(--primary-color);
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
      padding: 12px 15px;
    }

    .timeline-label {
      min-width: 100px;
      font-size: 13px;
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
</style>

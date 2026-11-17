<script>
  // The quiet date rail: count-proportional months, collision-filtered year
  // labels, a fixed pill thumb and a stage bubble. Navigation only — it never
  // writes the route filter.
  import { tick } from 'svelte';
  import { locale } from 'svelte-i18n';
  import { t } from '../lib/i18n.js';
  import { APP_CONSTANTS } from '../lib/constants.js';
  import { formatPeriodName } from '../lib/timeline.js';
  import { monthCount, monthsWithPhotos, snapMonthInDirection } from '../lib/dateScrubber.js';

  /** @type {{ model: any, descending: boolean, activeMonth: number|null,
   *   onScrub: (index: number) => void, onScrubEnd: () => void,
   *   onJump: (index: number) => void }} */
  const { model, descending, activeMonth, onScrub, onScrubEnd, onJump } = $props();

  const activeLocale = $derived($locale || 'en');

  let trackEl = $state(null);
  let dragging = $state(false);
  let hovered = $state(false);
  let thumbTop = $state(0);
  let yearMarks = $state([]);
  let resizeNonce = $state(0);

  const months = $derived(monthsWithPhotos(model, descending));
  const layoutKey = $derived(`${activeMonth}:${months.length}:${resizeNonce}:${descending}`);

  const monthName = (month) => {
    const monthKey = APP_CONSTANTS.MONTH_KEYS[month - 1];
    return $t(`ui.months.${monthKey}`, { locale: activeLocale, default: monthKey });
  };

  const periodLabel = (index) => formatPeriodName(index, monthName);
  const countLabel = (index) => {
    const count = monthCount(model, index);
    return count === 1
      ? $t('ui.photos_count_one', { locale: activeLocale, default: '1 photo' })
      : $t('ui.photos_count', {
          locale: activeLocale,
          values: { count },
          default: '{count} photos',
        });
  };

  const sliderLabel = $derived($t('ui.date_scrubber_label', { default: 'Date position' }));
  const valueText = $derived(
    activeMonth === null ? '' : `${periodLabel(activeMonth)} · ${countLabel(activeMonth)}`
  );

  function monthAt(clientY, target = null) {
    const direct = target?.closest?.('[data-month]');
    if (direct) return Number(direct.dataset.month);
    if (!trackEl) return null;
    const rect = trackEl.getBoundingClientRect();
    const hit = document.elementFromPoint(rect.left + rect.width / 2, clientY);
    const segment = hit?.closest?.('[data-month]');
    return segment ? Number(segment.dataset.month) : null;
  }

  function scrubToPointer(clientY, target = null) {
    const month = monthAt(clientY, target);
    if (month !== null) onScrub(month);
  }

  $effect(() => {
    if (!trackEl || activeMonth === null || layoutKey === null) return;
    const measure = async () => {
      await tick();
      if (!trackEl) return;
      const trackRect = trackEl.getBoundingClientRect();
      const activeEl = trackEl.querySelector(`[data-month="${activeMonth}"]`);
      if (activeEl) {
        const rect = activeEl.getBoundingClientRect();
        thumbTop = rect.top - trackRect.top + rect.height / 2;
      }
      const marks = [];
      let lastTop = -Infinity;
      let seenYear = null;
      for (const month of months) {
        const year = Math.floor(month / 12);
        if (year === seenYear) continue;
        seenYear = year;
        const el = trackEl.querySelector(`[data-month="${month}"]`);
        if (!el) continue;
        const top = el.getBoundingClientRect().top - trackRect.top;
        if (top - lastTop >= 16) {
          marks.push({ year, top });
          lastTop = top;
        }
      }
      yearMarks = marks;
    };
    measure();
  });

  $effect(() => {
    if (!trackEl) return;
    const observer = new ResizeObserver(() => {
      resizeNonce += 1;
    });
    observer.observe(trackEl);
    return () => observer.disconnect();
  });

  function handleKey(event) {
    const base = activeMonth ?? (descending ? model.maxIndex : model.minIndex);
    const towardTop = descending ? 1 : -1;
    const step = (direction, months) => {
      const next = snapMonthInDirection(model, base + direction * months, direction);
      if (next !== null) onJump(next);
    };
    const steps = {
      ArrowUp: [towardTop, 1],
      ArrowDown: [-towardTop, 1],
      PageUp: [towardTop, 12],
      PageDown: [-towardTop, 12],
    };
    if (Object.hasOwn(steps, event.key)) {
      event.preventDefault();
      step(steps[event.key][0], steps[event.key][1]);
      return;
    }
    if (event.key === 'Home') {
      event.preventDefault();
      onJump(descending ? model.maxIndex : model.minIndex);
    } else if (event.key === 'End') {
      event.preventDefault();
      onJump(descending ? model.minIndex : model.maxIndex);
    }
  }
</script>

<div class="date-rail" class:revealed={hovered || dragging} data-testid="date-rail">
  <div
    class="date-track"
    data-testid="date-track"
    bind:this={trackEl}
    role="slider"
    tabindex="0"
    aria-label={sliderLabel}
    aria-orientation="vertical"
    aria-valuemin={model.minIndex}
    aria-valuemax={model.maxIndex}
    aria-valuenow={activeMonth ?? model.minIndex}
    aria-valuetext={valueText}
    onpointerenter={() => (hovered = true)}
    onpointerleave={() => (hovered = false)}
    onpointerdown={(event) => {
      event.preventDefault();
      event.currentTarget.setPointerCapture(event.pointerId);
      dragging = true;
      scrubToPointer(event.clientY, event.target);
    }}
    onpointermove={(event) => {
      if (dragging) scrubToPointer(event.clientY);
    }}
    onpointerup={(event) => {
      if (!dragging) return;
      dragging = false;
      if (event.currentTarget.hasPointerCapture(event.pointerId)) {
        event.currentTarget.releasePointerCapture(event.pointerId);
      }
      onScrubEnd();
    }}
    onpointercancel={() => {
      if (!dragging) return;
      dragging = false;
      onScrubEnd();
    }}
    onkeydown={handleKey}
  >
    <div class="date-segments">
      {#each months as month (month)}
        <div
          class="date-segment"
          class:active={month === activeMonth}
          data-month={month}
          style:flex-grow={monthCount(model, month)}
        ></div>
      {/each}
    </div>
    <div class="date-thumb" style:top={`${thumbTop}px`} data-testid="date-thumb"></div>
  </div>

  <div class="date-years" aria-hidden="true">
    {#each yearMarks as mark (mark.year)}
      <span class="date-year" style:top={`${mark.top}px`}>{mark.year}</span>
    {/each}
  </div>

  {#if (hovered || dragging) && activeMonth !== null}
    <div
      class="date-bubble"
      style:top={`${thumbTop}px`}
      data-testid="date-bubble"
      aria-hidden="true"
    >
      <span class="date-bubble-month">{periodLabel(activeMonth)}</span>
      <span class="date-bubble-count">{countLabel(activeMonth)}</span>
    </div>
  {/if}
</div>

<style>
  .date-rail {
    position: fixed;
    top: calc(var(--header-height) + var(--space-4));
    right: 0;
    bottom: var(--space-4);
    z-index: var(--z-sticky);
    display: flex;
    width: 24px;
    justify-content: center;
  }

  .date-track {
    position: relative;
    width: 20px;
    height: 100%;
    cursor: grab;
    outline: none;
    touch-action: none;
    user-select: none;
  }

  .date-track:active {
    cursor: grabbing;
  }

  .date-track:focus-visible {
    border-radius: var(--radius-full);
    outline: 2px solid var(--primary-color);
    outline-offset: 3px;
  }

  .date-segments {
    position: absolute;
    top: 0;
    bottom: 0;
    left: 8px;
    display: flex;
    width: 4px;
    flex-direction: column;
  }

  .date-segment {
    flex-basis: 0;
    min-height: 1px;
    border-radius: var(--radius-full);
    background: color-mix(in oklch, var(--text-muted) 24%, transparent);
    transition: background var(--transition-fast);
  }

  .revealed .date-segment {
    background: color-mix(in oklch, var(--text-secondary) 38%, transparent);
  }

  .date-segment.active {
    background: var(--primary-color);
  }

  .date-thumb {
    position: absolute;
    left: 2px;
    width: 16px;
    height: 30px;
    margin-top: -15px;
    border: 1px solid var(--glass-border);
    border-radius: var(--radius-full);
    background: var(--surface-elevated);
    box-shadow: var(--shadow-medium);
    pointer-events: none;
    transition: top var(--transition-fast);
  }

  .date-thumb::after {
    position: absolute;
    top: 9px;
    left: 6px;
    width: 2px;
    height: 10px;
    border-radius: var(--radius-full);
    background: var(--primary-color);
    content: '';
  }

  .date-years {
    position: absolute;
    top: 0;
    right: 26px;
    bottom: 0;
    width: 40px;
    opacity: 0;
    pointer-events: none;
    transition: opacity var(--transition-fast);
  }

  .revealed .date-years {
    opacity: 1;
  }

  .date-year {
    position: absolute;
    right: 0;
    color: var(--text-secondary);
    font-family: var(--font-body);
    font-size: var(--font-xs);
    font-variant-numeric: tabular-nums;
    line-height: 1;
    transform: translateY(-50%);
  }

  .date-bubble {
    position: absolute;
    right: 72px;
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--glass-border);
    border-radius: var(--radius-md);
    background: var(--surface-elevated);
    box-shadow: var(--shadow-heavy);
    color: var(--text-primary);
    pointer-events: none;
    transform: translateY(-50%);
    white-space: nowrap;
  }

  .date-bubble-month {
    font-size: var(--font-sm);
    font-weight: var(--font-medium);
  }

  .date-bubble-count {
    color: var(--text-muted);
    font-size: var(--font-xs);
    font-variant-numeric: tabular-nums;
  }

  @media (width <= 768px) {
    .date-rail {
      display: none;
    }
  }
</style>

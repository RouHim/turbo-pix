<script>
  // The quiet date rail: a paper margin carrying decade ticks, a
  // count-proportional month track, a fixed pill thumb and a stage bubble.
  // Navigation only — it never writes the route filter.
  import { tick } from 'svelte';
  import { locale } from 'svelte-i18n';
  import { t } from '../lib/i18n.js';
  import { APP_CONSTANTS } from '../lib/constants.js';
  import { formatPeriodName } from '../lib/timeline.js';
  import {
    decadeStarts,
    monthCount,
    monthsWithPhotos,
    snapMonthInDirection,
  } from '../lib/dateScrubber.js';

  /** @type {{ model: any, descending: boolean, activeMonth: number|null,
   *   onScrub: (index: number) => void, onScrubEnd: () => void,
   *   onJump: (index: number) => void }} */
  const { model, descending, activeMonth, onScrub, onScrubEnd, onJump } = $props();

  const activeLocale = $derived($locale || 'en');
  const BUBBLE_MIN_TOP = 96;

  let railEl = $state(null);
  let trackEl = $state(null);
  let dragging = $state(false);
  let hovered = $state(false);
  let focused = $state(false);
  let dragPointerId = null;
  let thumbTop = $state(0);
  let trackOffset = $state(0);
  let decadeMarks = $state([]);
  let segmentMetrics = $state([]);
  let resizeNonce = $state(0);

  const months = $derived(monthsWithPhotos(model, descending));
  const decadeAnchors = $derived(decadeStarts(months));
  const activePosition = $derived(
    activeMonth === null || months.length === 0 ? 1 : Math.max(1, months.indexOf(activeMonth) + 1)
  );

  const monthName = (month) => {
    const monthKey = APP_CONSTANTS.MONTH_KEYS[month - 1];
    return $t(`ui.months.${monthKey}`, { locale: activeLocale, default: monthKey });
  };

  const shortMonth = $derived.by(() => {
    if (activeMonth === null) return '';
    const formatter = new Intl.DateTimeFormat(activeLocale, { month: 'short' });
    return formatter.format(new Date(2000, activeMonth % 12, 1));
  });
  const activeYear = $derived(activeMonth === null ? '' : String(Math.floor(activeMonth / 12)));

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
  const bubbleTop = $derived(Math.max(trackOffset + thumbTop, BUBBLE_MIN_TOP));

  /** The month whose segment center is nearest to the pointer. */
  function monthAt(clientY) {
    if (!trackEl || segmentMetrics.length === 0) return null;
    const y = clientY - trackEl.getBoundingClientRect().top;
    let bestMonth = null;
    let bestDistance = Infinity;
    for (const metric of segmentMetrics) {
      const distance = Math.abs(metric.center - y);
      if (distance < bestDistance) {
        bestDistance = distance;
        bestMonth = metric.month;
      }
    }
    return bestMonth;
  }

  function scrubToPointer(clientY) {
    const month = monthAt(clientY);
    if (month !== null) onScrub(month);
  }

  function endDrag(event = null) {
    if (!dragging) return;
    if (event?.pointerId !== undefined && event.pointerId !== dragPointerId) return;
    if (trackEl && dragPointerId !== null && trackEl.hasPointerCapture(dragPointerId)) {
      trackEl.releasePointerCapture(dragPointerId);
    }
    dragPointerId = null;
    dragging = false;
    onScrubEnd();
  }

  // Segment metrics and decade anchors only change with the track's layout;
  // the thumb follows the active month on every scrub tick.
  $effect(() => {
    const list = months;
    resizeNonce;
    if (!railEl || !trackEl || list.length === 0) return;
    const measure = async () => {
      await tick();
      if (!railEl || !trackEl) return;
      const trackRect = trackEl.getBoundingClientRect();
      const railRect = railEl.getBoundingClientRect();
      trackOffset = trackRect.top - railRect.top;
      const elements = trackEl.querySelectorAll('[data-month]');
      segmentMetrics = [...elements].map((element) => {
        const rect = element.getBoundingClientRect();
        return {
          month: Number(element.dataset.month),
          center: rect.top - trackRect.top + rect.height / 2,
        };
      });
      const marks = [];
      let lastTop = -Infinity;
      for (const anchor of decadeAnchors) {
        const element = trackEl.querySelector(`[data-month="${anchor.month}"]`);
        if (!element) continue;
        const top =
          element.getBoundingClientRect().top -
          trackRect.top +
          element.getBoundingClientRect().height / 2;
        if (top - lastTop >= 16) {
          marks.push({ decade: anchor.decade, top });
          lastTop = top;
        }
      }
      decadeMarks = marks;
    };
    measure();
  });

  $effect(() => {
    const active = activeMonth;
    const list = months;
    resizeNonce;
    if (!trackEl || active === null || list.length === 0) return;
    const measure = async () => {
      await tick();
      if (!trackEl) return;
      const activeEl = trackEl.querySelector(`[data-month="${active}"]`);
      if (!activeEl) return;
      const trackRect = trackEl.getBoundingClientRect();
      const rect = activeEl.getBoundingClientRect();
      thumbTop = rect.top - trackRect.top + rect.height / 2;
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
    const step = (direction, monthsToStep) => {
      const next = snapMonthInDirection(model, base + direction * monthsToStep, direction);
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

<svelte:window onblur={() => endDrag()} />

<div class="date-rail" class:dragging bind:this={railEl} data-testid="date-rail">
  <div class="date-caption" aria-hidden="true" data-testid="date-caption">
    <span class="date-caption-month">{activeMonth === null ? '—' : shortMonth}</span>
    <span class="date-caption-year">{activeYear}</span>
  </div>

  <div class="date-track-wrap">
    <div
      class="date-track"
      bind:this={trackEl}
      data-testid="date-track"
      role="slider"
      tabindex="0"
      aria-label={sliderLabel}
      aria-orientation="vertical"
      aria-valuemin={1}
      aria-valuemax={Math.max(1, months.length)}
      aria-valuenow={activePosition}
      aria-valuetext={valueText}
      onpointerenter={() => (hovered = true)}
      onpointerleave={() => (hovered = false)}
      onfocus={() => (focused = true)}
      onblur={() => (focused = false)}
      onpointerdown={(event) => {
        if (dragging) return;
        event.preventDefault();
        event.currentTarget.setPointerCapture(event.pointerId);
        dragPointerId = event.pointerId;
        dragging = true;
        scrubToPointer(event.clientY);
      }}
      onpointermove={(event) => {
        if (!dragging || event.pointerId !== dragPointerId) return;
        scrubToPointer(event.clientY);
      }}
      onpointerup={(event) => endDrag(event)}
      onpointercancel={(event) => endDrag(event)}
      onlostpointercapture={() => endDrag()}
      onkeydown={handleKey}
    >
      <div class="date-segments" data-testid="date-segments">
        {#each months as month (month)}
          <div
            class="date-segment"
            class:active={month === activeMonth}
            data-month={month}
            style:flex-grow={monthCount(model, month)}
          ></div>
        {/each}
      </div>
      <div
        class="date-thumb"
        class:ring={focused}
        style:top={`${thumbTop}px`}
        data-testid="date-thumb"
      ></div>
    </div>

    <div class="date-decades" aria-hidden="true">
      {#each decadeMarks as mark (mark.decade)}
        <span class="date-decade" style:top={`${mark.top}px`}>{mark.decade}</span>
      {/each}
    </div>
  </div>

  {#if (hovered || dragging || focused) && activeMonth !== null}
    <div
      class="date-bubble"
      style:top={`${bubbleTop}px`}
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
    top: var(--header-height);
    right: 0;
    bottom: 0;
    z-index: var(--z-sticky);
    box-sizing: border-box;
    display: flex;
    width: 56px;
    flex-direction: column;
  }

  .date-caption {
    display: flex;
    min-height: 44px;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 1px;
    border-bottom: 1px solid var(--divider-color);
    padding-block: var(--space-2);
    color: var(--text-secondary);
    font-family: var(--font-body);
    line-height: 1.1;
    text-align: center;
  }

  .date-caption-month {
    font-size: var(--font-xs);
    font-weight: var(--font-medium);
  }

  .date-caption-year {
    font-size: var(--font-xs);
    font-variant-numeric: tabular-nums;
  }

  .date-track-wrap {
    position: relative;
    flex: 1;
    min-height: 0;
    margin-block: var(--space-4);
  }

  .date-track {
    position: absolute;
    top: 0;
    right: 8px;
    bottom: 0;
    width: 20px;
    cursor: grab;
    outline: none;
    touch-action: none;
    user-select: none;
  }

  .date-track:active {
    cursor: grabbing;
  }

  .date-segments {
    position: absolute;
    top: 0;
    bottom: 0;
    left: 8px;
    display: flex;
    width: 4px;
    flex-direction: column;
    opacity: 0.32;
    transition: opacity var(--transition-medium);
  }

  .date-segment {
    flex-basis: 0;
    min-height: 1px;
    border-radius: var(--radius-full);
    background: color-mix(in oklch, var(--text-secondary) 70%, transparent);
    transition: background var(--transition-fast);
  }

  .date-track:hover .date-segment:not(.active) {
    background: color-mix(in oklch, var(--text-secondary) 85%, transparent);
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
  }

  .date-thumb.ring {
    box-shadow:
      0 0 0 2px var(--surface-color),
      0 0 0 4px var(--primary-color);
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

  .date-decades {
    position: absolute;
    top: 0;
    right: 22px;
    bottom: 0;
    width: 32px;
    pointer-events: none;
  }

  .date-decade {
    position: absolute;
    right: 0;
    color: var(--text-secondary);
    font-family: var(--font-body);
    font-size: var(--font-xs);
    font-variant-numeric: tabular-nums;
    line-height: 1;
    transform: translateY(-50%);
  }

  .date-decade::after {
    position: absolute;
    top: 50%;
    right: -4px;
    width: 2px;
    height: 1px;
    background: var(--text-muted);
    content: '';
  }

  .date-bubble {
    position: absolute;
    right: 64px;
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

  /* The revealed lane floats over the grid: a glass layer behind the rail's
     content, fading in on hover, keyboard focus and drag. The background sits
     on a pseudo-element so the backdrop-filter arrives with the opacity fade;
     the solid fallback wins under reduced transparency or no backdrop-filter. */
  .date-rail::before {
    position: absolute;
    inset: 0;
    z-index: -1;
    border-left: 1px solid var(--glass-border, var(--divider-color));
    background: var(--glass-bg);
    box-shadow: var(--shadow-light);
    content: '';
    opacity: 0;
    transition: opacity var(--transition-medium);
    backdrop-filter: blur(16px) saturate(1.5);
    -webkit-backdrop-filter: blur(16px) saturate(1.5);
  }

  .date-rail:is(:hover, :focus-within, .dragging)::before {
    opacity: 1;
  }

  @supports not (backdrop-filter: blur(1px)) {
    .date-rail::before {
      background: var(--surface-color);
    }
  }

  @media (prefers-reduced-transparency: reduce) {
    .date-rail::before {
      background: var(--surface-color);
      backdrop-filter: none;
      -webkit-backdrop-filter: none;
    }
  }

  /* Dormant until wanted: the marks keep a faint density spine as the only
     affordance; the caption, labels and thumb fade in on hover, keyboard
     focus or an active drag. Opacity only — display or visibility would drop
     the slider out of the tab order. */
  .date-caption,
  .date-decades,
  .date-thumb {
    opacity: 0;
    transition: opacity var(--transition-medium);
  }

  .date-rail:is(:hover, :focus-within, .dragging) .date-caption,
  .date-rail:is(:hover, :focus-within, .dragging) .date-decades,
  .date-rail:is(:hover, :focus-within, .dragging) .date-thumb,
  .date-rail:is(:hover, :focus-within, .dragging) .date-segments {
    opacity: 1;
  }

  @media (prefers-reduced-motion: reduce) {
    .date-rail::before,
    .date-caption,
    .date-decades,
    .date-thumb,
    .date-segments,
    .date-segment {
      transition: none;
    }
  }

  @media (width <= 768px) {
    .date-rail {
      display: none;
    }
  }
</style>

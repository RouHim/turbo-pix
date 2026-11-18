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
   *   pending: boolean, failedMonth: number|null,
   *   onScrub: (index: number) => void, onScrubEnd: () => void,
   *   onCancel: () => void, onJump: (index: number) => void }} */
  const {
    model,
    descending,
    activeMonth,
    pending,
    failedMonth,
    onScrub,
    onScrubEnd,
    onCancel,
    onJump,
  } = $props();

  const activeLocale = $derived($locale || 'en');
  const BUBBLE_MIN_TOP = 96;

  let railEl = $state(null);
  let trackEl = $state(null);
  let dragging = $state(false);
  let hovered = $state(false);
  let hoveredMonth = $state(null);
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

  const sliderLabel = $derived($t('ui.date_scrubber_label', { default: 'Jump to a month' }));
  const newestLabel = $derived($t('ui.date_scrubber_newest', { default: 'Back to newest' }));
  const retryLabel = $derived($t('ui.date_scrubber_retry', { default: 'Retry' }));
  const valueText = $derived(
    activeMonth === null ? '' : `${periodLabel(activeMonth)} · ${countLabel(activeMonth)}`
  );
  const newestIndex = $derived(descending ? model.maxIndex : model.minIndex);
  const centerOf = (month) =>
    segmentMetrics.find((metric) => metric.month === month)?.center ?? thumbTop;
  const bubbleMonth = $derived(
    failedMonth ?? (pending || dragging ? activeMonth : (hoveredMonth ?? activeMonth))
  );
  const jumpLabel = $derived(
    activeMonth === null
      ? ''
      : $t('ui.date_scrubber_jump_to', {
          locale: activeLocale,
          values: { period: periodLabel(activeMonth) },
          default: 'Jump to {period}…',
        })
  );
  const bubbleTop = $derived(
    Math.max(
      trackOffset +
        (failedMonth !== null
          ? centerOf(failedMonth)
          : pending
            ? thumbTop
            : hoveredMonth !== null && !dragging
              ? centerOf(hoveredMonth)
              : thumbTop),
      BUBBLE_MIN_TOP
    )
  );

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

  function cancelDrag() {
    if (!dragging) return;
    if (trackEl && dragPointerId !== null && trackEl.hasPointerCapture(dragPointerId)) {
      trackEl.releasePointerCapture(dragPointerId);
    }
    dragPointerId = null;
    dragging = false;
    onCancel();
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
    if (event.key === 'Escape') {
      event.preventDefault();
      cancelDrag();
      hovered = false;
      hoveredMonth = null;
      return;
    }
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
  <button
    type="button"
    class="date-caption"
    data-testid="date-caption"
    aria-label={newestLabel}
    title={newestLabel}
    onpointerenter={() => (hovered = true)}
    onpointerleave={() => (hovered = false)}
    onclick={() => onJump(newestIndex)}
  >
    <span class="date-caption-month">{activeMonth === null ? '—' : shortMonth}</span>
    <span class="date-caption-year">{activeYear}</span>
  </button>

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
      aria-valuenow={months.length - activePosition + 1}
      aria-valuetext={valueText}
      onpointerenter={(event) => {
        hovered = true;
        hoveredMonth = monthAt(event.clientY);
      }}
      onpointerleave={() => {
        hovered = false;
        hoveredMonth = null;
      }}
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
        if (dragging && event.pointerId === dragPointerId) {
          scrubToPointer(event.clientY);
          return;
        }
        hoveredMonth = monthAt(event.clientY);
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
            class:ghost={month === hoveredMonth && month !== activeMonth}
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

  {#if (hovered || dragging || focused || pending || failedMonth !== null) && bubbleMonth !== null}
    <div class="date-bubble" style:top={`${bubbleTop}px`} data-testid="date-bubble">
      {#if failedMonth !== null}
        <span class="date-bubble-month">{periodLabel(failedMonth)}</span>
        <span class="date-bubble-count">{countLabel(failedMonth)}</span>
        <button type="button" class="date-bubble-retry" onclick={() => onJump(failedMonth)}>
          {retryLabel}
        </button>
      {:else if pending}
        <span class="date-bubble-month">{jumpLabel}</span>
        <span class="date-bubble-count">{countLabel(activeMonth)}</span>
        <span class="date-bubble-dots" aria-hidden="true">
          <span></span>
          <span></span>
          <span></span>
        </span>
      {:else}
        <span class="date-bubble-month">{periodLabel(bubbleMonth)}</span>
        <span class="date-bubble-count">{countLabel(bubbleMonth)}</span>
      {/if}
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
    width: calc(100% - var(--space-2));
    min-height: 44px;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 1px;
    margin: var(--space-1) auto 0;
    border: 1px solid var(--glass-border, var(--divider-color));
    border-radius: var(--radius-full);
    padding-block: var(--space-2);
    background: var(--glass-bg);
    color: var(--text-primary);
    cursor: pointer;
    font-family: var(--font-body);
    line-height: 1.1;
    text-align: center;
    pointer-events: auto;
    transition:
      color var(--transition-medium),
      border-color var(--transition-medium);
    backdrop-filter: blur(16px) saturate(1.5);
    -webkit-backdrop-filter: blur(16px) saturate(1.5);
  }

  .date-caption:focus-visible {
    outline: 2px solid var(--primary-color);
    outline-offset: -2px;
  }

  @supports not (backdrop-filter: blur(1px)) {
    .date-caption {
      background: var(--surface-color);
    }
  }

  @media (prefers-reduced-transparency: reduce) {
    .date-caption {
      background: var(--surface-color);
      backdrop-filter: none;
      -webkit-backdrop-filter: none;
    }
  }

  .date-caption-month {
    color: var(--text-secondary);
    font-size: var(--font-xs);
    font-weight: var(--font-medium);
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }

  .date-caption-year {
    font-size: var(--font-base);
    font-weight: var(--font-semibold);
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
    pointer-events: auto;
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
  }

  .date-segment {
    flex-basis: 0;
    min-height: 1px;
    border-radius: var(--radius-full);
    background: color-mix(in oklch, var(--text-secondary) 70%, transparent);
    opacity: 0.32;
    transition:
      opacity var(--transition-medium),
      background var(--transition-fast);
  }

  .date-track:hover .date-segment:not(.active):not(.ghost) {
    background: color-mix(in oklch, var(--text-secondary) 85%, transparent);
  }

  .date-segment.active {
    min-height: 3px;
    background: var(--primary-color);
    opacity: 1;
  }

  .date-segment.ghost {
    background: color-mix(in oklch, var(--primary-color) 55%, transparent);
    opacity: 1;
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

  .date-bubble-retry {
    border: 0;
    padding: 0 2px;
    background: transparent;
    color: var(--primary-color);
    cursor: pointer;
    font: inherit;
    font-size: var(--font-xs);
    text-decoration: underline;
    pointer-events: auto;
  }

  .date-bubble-retry:focus-visible {
    outline: 2px solid var(--primary-color);
    outline-offset: 2px;
  }

  .date-bubble-dots {
    display: inline-flex;
    gap: 3px;
  }

  .date-bubble-dots span {
    width: 4px;
    height: 4px;
    border-radius: var(--radius-full);
    background: var(--primary-color);
    animation: bubble-dot 1.2s ease-in-out infinite;
  }

  .date-bubble-dots span:nth-child(2) {
    animation-delay: 0.15s;
  }

  .date-bubble-dots span:nth-child(3) {
    animation-delay: 0.3s;
  }

  @keyframes bubble-dot {
    0%,
    60%,
    100% {
      opacity: 0.5;
      transform: translateY(0);
    }

    30% {
      opacity: 1;
      transform: translateY(-2px);
    }
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
  .date-caption {
    opacity: 1;
  }

  .date-decades,
  .date-thumb {
    opacity: 0;
    transition: opacity var(--transition-medium);
  }

  .date-rail:is(:hover, :focus-within, .dragging) .date-caption {
    border-bottom-color: var(--divider-color);
    color: var(--text-primary);
  }

  .date-rail:is(:hover, :focus-within, .dragging) .date-decades,
  .date-rail:is(:hover, :focus-within, .dragging) .date-thumb,
  .date-rail:is(:hover, :focus-within, .dragging) .date-segment {
    opacity: 1;
  }

  @media (prefers-reduced-motion: reduce) {
    .date-rail::before,
    .date-caption,
    .date-decades,
    .date-thumb,
    .date-segments,
    .date-segment,
    .date-bubble-dots span {
      transition: none;
      animation: none;
    }
  }

  @media (width <= 768px) {
    .date-rail {
      display: none;
    }
  }
</style>

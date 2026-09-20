<script>
  import { tick, untrack } from 'svelte';
  import { locale } from 'svelte-i18n';
  import { t } from '../lib/i18n.js';
  import { APP_CONSTANTS } from '../lib/constants.js';
  import {
    ZOOM_STEP,
    buildColumns,
    chooseUnit,
    clampBound,
    clampView,
    createView,
    ensureSelectionVisible,
    indexFromX,
    panView,
    placeLabels,
    selectionZoneAtX,
    translateSelection,
    xFromIndex,
    zoomToRange,
    zoomView,
  } from '../lib/timelineLayout.js';
  import {
    MONTHS_PER_DECADE,
    MONTHS_PER_YEAR,
    clampIndexToModel,
    countInRange,
    formatPeriodName,
    formatSelectionLabel,
    normalizeSelection,
  } from '../lib/timeline.js';
  import Icon from './Icon.svelte';

  /** @type {{ model: any, selection: { startIndex: number, endIndex: number } | null, onchange: (selection: any, options?: { commit?: boolean }) => void, resetNonce?: number }} */
  const { model, selection, onchange, resetNonce = 0 } = $props();

  const activeLocale = $derived($locale || 'en');
  const LANE_HEIGHT = 56;
  const DRAG_THRESHOLD_PX = 4;

  let laneEl = $state(null);
  let rulerEl = $state(null);
  let probeEl = $state(null);
  let width = $state(0);
  let view = $state(null);
  let drag = $state(null);
  let hoveredColumn = $state(null);
  let focusedColumnStart = $state(null);
  let suppressClick = false;
  let measureContext = null;
  // Set while a keyboard focus move owns the view change it makes: the
  // selection-following effect further down must not undo that pan (see
  // `focusColumn`).
  let reframeSuppressed = false;

  const monthName = (month) => {
    const monthKey = APP_CONSTANTS.MONTH_KEYS[month - 1];
    return $t(`ui.months.${monthKey}`, { locale: activeLocale, default: monthKey });
  };

  const periodName = (index) => formatPeriodName(index, monthName);

  const format = $derived({
    allDates: $t('ui.all_dates', { default: 'All Dates' }),
    monthName,
    periodName,
    rangeTemplate: (start, end) =>
      $t('ui.timeline_range_label', { values: { start, end }, default: '{start} – {end}' }),
  });

  // Rendered text is a hair wider than canvas text, hence the safety margin the
  // layout already applies on top of this measurement.
  const measure = (text) => {
    if (!probeEl) return text.length * 8;
    measureContext ??= document.createElement('canvas').getContext('2d');
    measureContext.font = window.getComputedStyle(probeEl).font;
    return measureContext.measureText(text).width;
  };

  const unit = $derived(chooseUnit(view?.scale ?? 1));
  const columns = $derived(
    view === null ? [] : buildColumns({ unit, view, width, model, format, countInRange })
  );
  const placedColumns = $derived(placeLabels(columns, { width, measure }));
  const columnMax = $derived(columns.reduce((max, column) => Math.max(max, column.count), 0) || 1);
  const effectiveSelection = $derived(drag?.selection ?? selection);

  // FR-009 gives draggable bounds and translation to an active *range* only. A
  // single period — one month, or one whole year, the two shapes the route
  // collapses into a bare `year`/`month` — is not a range, so a press on its
  // overlay starts a new brush instead of a translation. Without that, a
  // whole-year selection (which paints every month the year view can show)
  // would swallow every brush and the year filter could never be narrowed by
  // dragging. The handles keep their own gesture either way.
  const isRange = $derived.by(() => {
    const range = effectiveSelection;
    if (range === null || range.endIndex <= range.startIndex) return false;
    const wholeYear =
      range.endIndex - range.startIndex === MONTHS_PER_YEAR - 1 &&
      range.startIndex % MONTHS_PER_YEAR === 0;
    return !wholeYear;
  });

  // `selectionFromFilter` keeps a bare whole-year period's own lower boundary,
  // so a `?year=2012` deep link can start left of the first column. Paint the
  // overlay clamped into the lane: the start handle must stay visible and
  // grabbable, while `aria-valuetext` still names the true bound.
  const overlay = $derived.by(() => {
    if (view === null || effectiveSelection === null || width <= 0) return null;
    const left = Math.min(Math.max(xFromIndex(effectiveSelection.startIndex, view), 0), width);
    const right = Math.min(Math.max(xFromIndex(effectiveSelection.endIndex + 1, view), 0), width);
    return { left, width: right - left };
  });

  const photoCountLabel = (count) =>
    count === 0
      ? $t('ui.timeline_no_photos_month', { default: 'No photos' })
      : $t('ui.photos_count', { values: { count }, default: '{count} photos' });

  const rowCount = (column) => photoCountLabel(column.count);

  // Every period announcement names the period *and* what it holds (FR-014,
  // SC-005): columns say it in `aria-label`, handles in `aria-valuetext`.
  const periodAnnouncement = (index) =>
    `${periodName(index)}, ${photoCountLabel(countInRange(model, index, index))}`;

  const statusText = $derived.by(() => {
    const column = hoveredColumn ?? columns.find((c) => c.gridStart === focusedColumnStart);
    if (column) return `${periodName(column.gridStart)}, ${rowCount(column)}`;
    const label = formatSelectionLabel(effectiveSelection, format);
    if (effectiveSelection !== null) return label;
    return $t('ui.timeline_span_summary', {
      values: { range: label, count: model.total },
      default: '{range} · {count} photos',
    });
  });

  const pointerX = (event, element) => event.clientX - element.getBoundingClientRect().left;

  const nextSelectionFor = (x) => {
    const base = drag.baseSelection;
    if (drag.zone === 'brush') {
      return normalizeSelection(drag.anchorIndex, clampIndexToModel(model, indexFromX(x, view)));
    }
    if (drag.zone === 'body') {
      // The span never changes: the selection follows the pointer in whole
      // months, not in the fractional months a raw pixel ratio would give.
      return translateSelection(base, Math.round((x - drag.startX) / view.scale), model);
    }
    const bound = clampBound(indexFromX(x, view), base, drag.zone, model);
    return drag.zone === 'start'
      ? { startIndex: bound, endIndex: base.endIndex }
      : { startIndex: base.startIndex, endIndex: bound };
  };

  // FR-007: a period without photos is never selectable, so no gesture may
  // write a selection that is exactly one zero-photo month — neither the live
  // scrub nor the committed value. A range of two months or more stays legal
  // (it may merely contain empty months), and a route that already names such
  // a period is left to the route.
  const isSelectableSelection = (next) =>
    next === null ||
    next.endIndex > next.startIndex ||
    countInRange(model, next.startIndex, next.startIndex) > 0;

  const writeSelection = (next, commit) => {
    if (!isSelectableSelection(next)) return;
    onchange(next, { commit });
  };

  const beginDrag = (zone, event) => {
    const element = zone === 'pan' ? rulerEl : laneEl;
    if (element === null || view === null || model.length === 0) return;
    const x = pointerX(event, element);
    // A pan leaves the route selection alone, so the overlay keeps the `??`
    // fallback and stays visible while the ruler moves under it.
    const baseSelection = zone === 'brush' ? null : effectiveSelection;
    drag = {
      zone,
      pointerId: event.pointerId,
      startX: x,
      startY: event.clientY,
      moved: false,
      captured: false,
      aborted: false,
      baseSelection,
      previousSelection: selection,
      baseView: view,
      anchorIndex: clampIndexToModel(model, indexFromX(x, view)),
      selection: baseSelection,
    };
  };

  const startPan = (event) => {
    if (event.button !== 0) return;
    suppressClick = false;
    beginDrag('pan', event);
  };

  const startLaneGesture = (event) => {
    if (event.button !== 0) return;
    suppressClick = false;
    if (laneEl === null || view === null || model.length === 0) return;
    const x = pointerX(event, laneEl);
    const zone = selectionZoneAtX(x, { selection: effectiveSelection, view, width }) ?? 'brush';
    beginDrag(isRange ? zone : 'brush', event);
  };

  // A handle drags its own bound instead of relying on the lane's
  // ±HANDLE_HIT_PX zone, so the grab is exact, and a press that starts on a
  // handle is never also read as a brush.
  const startHandleGesture = (event, bound) => {
    if (event.button !== 0) return;
    event.stopPropagation();
    suppressClick = false;
    beginDrag(bound, event);
  };

  const handlePointerMove = (event) => {
    // An aborted gesture keeps its capture until the pointer is released so the
    // release cannot reach a column (see the Escape handler), but it must not
    // follow the pointer any more.
    if (drag === null || drag.aborted || event.pointerId !== drag.pointerId) return;
    const element = drag.zone === 'pan' ? rulerEl : laneEl;
    if (element === null || view === null || model.length === 0) return;
    const x = pointerX(event, element);

    if (!drag.moved) {
      const far =
        Math.abs(x - drag.startX) >= DRAG_THRESHOLD_PX ||
        Math.abs(event.clientY - drag.startY) >= DRAG_THRESHOLD_PX;
      if (!far) return;
      drag.moved = true;
      // Capture only once the gesture is a real drag: capturing on pointerdown
      // retargets the compatibility click too, so a plain column click would
      // never reach the column.
      element.setPointerCapture(event.pointerId);
      drag.captured = true;
    }

    if (drag.zone === 'pan') {
      view = panView({ view: drag.baseView, deltaPx: x - drag.startX, width, model });
      return;
    }

    const next = nextSelectionFor(x);
    drag.selection = next;
    writeSelection(next, false);
  };

  const endDrag = () => {
    if (drag === null) return;
    const element = drag.zone === 'pan' ? rulerEl : laneEl;
    const { pointerId, captured } = drag;
    drag = null;
    if (captured && element !== null && element.hasPointerCapture(pointerId)) {
      element.releasePointerCapture(pointerId);
    }
  };

  const endGesture = (event) => {
    if (drag === null || event.pointerId !== drag.pointerId) return;
    const { moved, zone, selection: dragged, aborted } = drag;
    endDrag();
    if (!moved) return;
    // Review Focus 1: a drag that ends over a column must not also activate it.
    // A ruler pan is exempt: it captures the ruler, so its compatibility click
    // can never reach a column and a flag set here would instead swallow the
    // next keyboard activation.
    if (zone !== 'pan') suppressClick = true;
    // An Escape-cancelled gesture has already committed the pre-drag selection;
    // committing the last dragged value now would overwrite it.
    if (zone !== 'pan' && dragged !== null && !aborted) writeSelection(dragged, true);
  };

  // A gesture that ends over a column retargets the compatibility click to the
  // captured lane, so the column's own handler never consumes `suppressClick`
  // and a following keyboard activation would be discarded once. The lane's
  // click runs after a column's own handler, so this cannot unmask a column
  // click either.
  const clearSuppressClick = () => {
    suppressClick = false;
  };

  const activateColumn = (column) => {
    if (suppressClick) {
      suppressClick = false;
      return;
    }
    if (column.count === 0 || view === null || model.length === 0) return;

    if (unit === MONTHS_PER_DECADE) {
      view = zoomToRange(
        { startIndex: column.gridStart, endIndex: column.gridStart + MONTHS_PER_DECADE - 1 },
        width,
        model
      );
      return;
    }

    onchange({ startIndex: column.startIndex, endIndex: column.endIndex }, { commit: true });
    if (unit === MONTHS_PER_YEAR) {
      // Drill in so months become reachable in three interactions.
      view = zoomToRange(
        { startIndex: column.startIndex, endIndex: column.endIndex },
        width,
        model
      );
    }
  };

  // The `unit`-aligned grid the roving focus moves along. A period is addressed
  // by its grid start, which for the first column can sit before the model
  // begins (a decade that only partly exists).
  const alignedStart = (index) => Math.floor(index / unit) * unit;
  const gridStartMin = $derived(model.length === 0 ? null : alignedStart(model.minIndex));
  const gridStartMax = $derived(model.length === 0 ? null : alignedStart(model.maxIndex));

  const columnPhotoCount = (gridStart) =>
    countInRange(
      model,
      Math.max(gridStart, model.minIndex),
      Math.min(gridStart + unit - 1, model.maxIndex)
    );

  // Arrow/Home/End walk the periods a user can *act* on: a period without
  // photos announces itself but ignores activation (FR-007, `aria-disabled`),
  // and it cannot be selected either, so navigation lands on periods with
  // photos instead of parking on one that swallows Enter. `null` means the
  // direction holds none.
  const seekFocusedStart = (from, step) => {
    const last = step > 0 ? gridStartMax : gridStartMin;
    if (from === null || last === null) return null;
    for (
      let candidate = from;
      step > 0 ? candidate <= last : candidate >= last;
      candidate += step
    ) {
      if (columnPhotoCount(candidate) > 0) return candidate;
    }
    return null;
  };

  // Move the roving focus to `gridStart` and hold that period on screen. The
  // reveal pan is this interaction's own view change, so the selection-
  // following effect must leave it alone: undoing it would drop the column out
  // of the grid and take the focus with it.
  const focusColumn = async (gridStart) => {
    focusedColumnStart = gridStart;
    if (view !== null && width > 0 && model.length > 0) {
      const framed = ensureSelectionVisible(
        {
          startIndex: Math.max(gridStart, model.minIndex),
          endIndex: Math.min(gridStart + unit - 1, model.maxIndex),
        },
        view,
        width,
        model
      );
      if (framed.scale !== view.scale || framed.origin !== view.origin) {
        reframeSuppressed = true;
        view = framed;
      }
    }
    await tick();
    const element = laneEl?.querySelector(`.timeline-column[data-period-start="${gridStart}"]`);
    if (element instanceof HTMLElement) element.focus();
  };

  // A committed keyboard change re-renders the grid — a drill changes the
  // column unit and an extended range can reframe the view — so the column that
  // held the focus is unmounted with it and the focus drops to `<body>`. Put it
  // back on the period the change landed on, snapped to the grid that now
  // exists and stepped to one with photos (the arrows' own rule).
  const restoreColumnFocus = async (index) => {
    await tick();
    if (document.activeElement?.closest?.('.timeline-column')) return;
    const aligned = alignedStart(index);
    await focusColumn(seekFocusedStart(aligned, 1) ?? aligned);
  };

  const handleColumnKeydown = (event, column) => {
    if (event.key === 'Enter' || event.key === ' ' || event.key === 'Spacebar') {
      // Both forms prevent the button's own click: the plain one because it
      // runs the activation itself (the same function the click runs, drill-in
      // included), the shifted one because it must not also select the single
      // period underneath the range it just extended to.
      event.preventDefault();
      if (event.shiftKey) {
        // Shift extends the selection to this period: the keyboard twin of a
        // brush drag.
        writeSelection(
          normalizeSelection(selection?.startIndex ?? column.startIndex, column.endIndex),
          true
        );
      } else {
        activateColumn(column);
      }
      restoreColumnFocus(column.gridStart);
      return;
    }

    const step = event.key === 'ArrowRight' ? 1 : event.key === 'ArrowLeft' ? -1 : 0;
    let target;
    if (step !== 0) target = seekFocusedStart(column.gridStart + step * unit, step);
    else if (event.key === 'Home') target = seekFocusedStart(gridStartMin, 1);
    else if (event.key === 'End') target = seekFocusedStart(gridStartMax, -1);
    else return;

    event.preventDefault();
    if (target === null || target === column.gridStart) return;
    focusColumn(target);
  };

  // Handles are sliders: arrows move the bound one month at a time through
  // `clampBound` (a bound never crosses the other, so the range never inverts
  // and never leaves the model span), Home/End go to the model ends, and every
  // move commits so the grid follows (SC-003).
  const handleBoundKeydown = (event, bound) => {
    const base = effectiveSelection;
    if (base === null || model.length === 0) return;

    let index;
    if (event.key === 'ArrowLeft' || event.key === 'ArrowRight') {
      const current = bound === 'start' ? base.startIndex : base.endIndex;
      index = current + (event.key === 'ArrowRight' ? 1 : -1);
    } else if (event.key === 'Home') index = model.minIndex;
    else if (event.key === 'End') index = model.maxIndex;
    else return;

    event.preventDefault();
    const moved =
      bound === 'start'
        ? { startIndex: clampBound(index, base, 'start', model), endIndex: base.endIndex }
        : { startIndex: base.startIndex, endIndex: clampBound(index, base, 'end', model) };
    if (moved.startIndex === base.startIndex && moved.endIndex === base.endIndex) return;
    // The same refusal rule as every other write: a bound walked onto the
    // opposite bound must not be the way to select one zero-photo month.
    writeSelection(moved, true);
  };

  const handleWheel = (event) => {
    if (view === null || width <= 0) return;
    event.preventDefault();
    const element = laneEl ?? rulerEl;
    if (element === null) return;

    if (event.shiftKey || Math.abs(event.deltaX) > Math.abs(event.deltaY)) {
      view = panView({
        view,
        deltaPx: -(event.shiftKey ? event.deltaY : event.deltaX),
        width,
        model,
      });
      return;
    }

    view = zoomView({
      view,
      factor: Math.exp(-event.deltaY * 0.002),
      anchorPx: pointerX(event, element),
      width,
      model,
    });
  };

  const zoomIn = () => {
    if (view === null || model.length === 0) return;
    view = zoomView({ view, factor: ZOOM_STEP, anchorPx: width / 2, width, model });
  };

  const zoomOut = () => {
    if (view === null || model.length === 0) return;
    view = zoomView({ view, factor: 1 / ZOOM_STEP, anchorPx: width / 2, width, model });
  };

  const fitAll = () => {
    view = createView(width, model);
  };

  // The lane is `display: none` at the mobile breakpoint and then reports a
  // width of 0. Keeping the last non-zero width means a hide/show round trip
  // cannot throw the user's zoom away (clampView pins a 0-width view to fit all).
  $effect(() => {
    if (!laneEl) return;
    const observer = new ResizeObserver((entries) => {
      const entry = entries[entries.length - 1];
      const measured = entry ? entry.contentRect.width : 0;
      if (measured > 0) width = measured;
    });
    observer.observe(laneEl);
    return () => observer.disconnect();
  });

  // Re-clamp on model/width/view changes, but assign only when something moved:
  // clampView returns a fresh object, so an unconditional assign re-triggers
  // this effect forever. The first view a restored selection builds is framed
  // on that selection instead of the whole span (SC-002: a `?year=2012` deep
  // link opens on that year's months, so the month it names is one activation
  // away), which is why `selection` is read before the guard.
  $effect(() => {
    const restored = selection;
    if (width <= 0 || model.length === 0) return;
    let next;
    if (view === null) {
      next = restored === null ? createView(width, model) : zoomToRange(restored, width, model);
    } else {
      next = clampView(view, width, model);
    }
    if (view === null || view.scale !== next.scale || view.origin !== next.origin) {
      view = next;
    }
  });

  // Reset only when the container bumps the nonce: width and model are pulled
  // untracked, otherwise every resize would reset the zoom. Mount is not a
  // reset — the view built above is already the right one for a deep link, and
  // re-creating it here from a not-yet-measured width would throw the restored
  // selection's framing away.
  let resetHandled = resetNonce;
  $effect(() => {
    const nonce = resetNonce;
    if (nonce === resetHandled) return;
    resetHandled = nonce;
    untrack(() => {
      drag = null;
      view = createView(width, model);
    });
  });

  // FR-010: keep the selection on screen — never fight a drag in progress or a
  // keyboard focus move, and leave an already visible selection's view
  // untouched. (`reframeSuppressed` is read before the guards, and is one-shot:
  // it belongs to the single view change `focusColumn` just made.)
  $effect(() => {
    const dragged = drag;
    const current = view;
    const visible = selection;
    const ownedByFocusMove = reframeSuppressed;
    reframeSuppressed = false;
    const next =
      visible === null || current === null
        ? current
        : ensureSelectionVisible(visible, current, width, model);
    if (dragged !== null || ownedByFocusMove || current === null || next === current) return;
    if (next.scale !== current.scale || next.origin !== current.origin) view = next;
  });

  // Roving tabindex: exactly one column is tabbable, so the lane can be entered
  // from the keyboard, without pretending a column is "focused" (the status row
  // only names a column once it really is hovered or focused). The remembered
  // column wins; one the current grid no longer holds (arrow to a month, then
  // fit-all) falls back to the column of the current selection, and finally to
  // the first visible one. Either of those may be a period without photos —
  // those stay focusable on purpose (`aria-disabled`, never `disabled`).
  const tabbableStart = $derived.by(() => {
    if (columns.length === 0) return null;
    const preferred = [focusedColumnStart, alignedStart(selection?.startIndex ?? model.minIndex)];
    return (
      preferred.find((start) => columns.some((column) => column.gridStart === start)) ??
      columns[0].gridStart
    );
  });

  // Escape abandons the gesture and puts the pre-drag view/selection back — as
  // a committed action, so the aborted gesture leaves one history entry rather
  // than the replaced one the live scrub wrote. The gesture stays open (and
  // keeps its pointer capture) until the pointer is released: releasing the
  // capture here would send the release's compatibility click to whatever
  // column the pointer happens to be over, and that activation would filter on
  // top of the selection Escape just restored. Kept open, the release takes the
  // ordinary drag-end path — click suppressed, no second commit.
  $effect(() => {
    if (drag === null) return;
    const onKeyDown = (event) => {
      if (event.key !== 'Escape' || drag.aborted) return;
      event.preventDefault();
      const { zone, previousSelection, baseView } = drag;
      drag.aborted = true;
      drag.selection = previousSelection;
      if (zone === 'pan') view = baseView;
      else onchange(previousSelection, { commit: true });
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  });
</script>

<div
  class="timeline-selector"
  role="group"
  aria-label={$t('ui.timeline_overview_label', { default: 'Timeline overview' })}
>
  <span class="timeline-ruler-label timeline-label-probe" bind:this={probeEl} aria-hidden="true"
    >MMM 0000</span
  >

  <!-- The ruler is a pointer-only pan surface inside the role="group" selector;
       its labels are decorative, so it carries no ARIA role of its own. -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="timeline-ruler"
    bind:this={rulerEl}
    onpointerdown={startPan}
    onpointermove={handlePointerMove}
    onpointerup={endGesture}
    onpointercancel={endGesture}
    onwheel={handleWheel}
  >
    {#each placedColumns as column (column.gridStart)}
      {#if column.labelX !== null}
        <span class="timeline-ruler-label" style="left: {column.labelX}px">{column.label}</span>
      {/if}
    {/each}
  </div>

  <!-- The lane only *hosts* the interactive columns and the slider handles; the
       pointer gesture is a zoom/brush surface, not a widget of its own, and its
       click handler only clears the post-drag suppression flag. -->
  <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
  <div
    class="timeline-lane"
    bind:this={laneEl}
    onpointerdown={startLaneGesture}
    onpointermove={handlePointerMove}
    onpointerup={endGesture}
    onpointercancel={endGesture}
    onwheel={handleWheel}
    onclick={clearSuppressClick}
  >
    {#each placedColumns as column (column.gridStart)}
      <button
        type="button"
        class="timeline-column"
        class:active={effectiveSelection !== null &&
          column.startIndex <= effectiveSelection.endIndex &&
          column.endIndex >= effectiveSelection.startIndex}
        class:empty={column.count === 0}
        class:hovered={hoveredColumn?.gridStart === column.gridStart}
        data-period-start={column.gridStart}
        data-unit={unit}
        style="left: {column.x}px; width: {column.width}px;"
        aria-label={`${periodName(column.gridStart)}, ${rowCount(column)}`}
        aria-pressed={effectiveSelection !== null &&
          column.startIndex <= effectiveSelection.endIndex &&
          column.endIndex >= effectiveSelection.startIndex}
        aria-disabled={column.count === 0}
        tabindex={column.gridStart === tabbableStart ? 0 : -1}
        onclick={() => activateColumn(column)}
        onkeydown={(event) => handleColumnKeydown(event, column)}
        onmouseenter={() => (hoveredColumn = column)}
        onmouseleave={() => (hoveredColumn = null)}
        onfocus={() => {
          hoveredColumn = column;
          focusedColumnStart = column.gridStart;
        }}
        onblur={() => (hoveredColumn = null)}
      >
        <span
          class="timeline-column-bar"
          style="height: {column.count === 0
            ? 0
            : Math.max(3, Math.round((column.count / columnMax) * (LANE_HEIGHT - 12)))}px"
        ></span>
      </button>
    {/each}

    {#if overlay !== null}
      <div class="timeline-selection" style="left: {overlay.left}px; width: {overlay.width}px">
        <!-- Pointer- and keyboard-operable bounds: drag to move a bound, or
             focus it and use arrows/Home/End, which announce the bound's period
             and photo count through the slider's value text. -->
        <div
          class="timeline-handle start"
          role="slider"
          tabindex="0"
          aria-label={$t('ui.timeline_range_start', { default: 'Range start' })}
          aria-valuemin={model.minIndex}
          aria-valuemax={model.maxIndex}
          aria-valuenow={effectiveSelection.startIndex}
          aria-valuetext={periodAnnouncement(effectiveSelection.startIndex)}
          onpointerdown={(event) => startHandleGesture(event, 'start')}
          onkeydown={(event) => handleBoundKeydown(event, 'start')}
        ></div>
        <div
          class="timeline-handle end"
          role="slider"
          tabindex="0"
          aria-label={$t('ui.timeline_range_end', { default: 'Range end' })}
          aria-valuemin={model.minIndex}
          aria-valuemax={model.maxIndex}
          aria-valuenow={effectiveSelection.endIndex}
          aria-valuetext={periodAnnouncement(effectiveSelection.endIndex)}
          onpointerdown={(event) => startHandleGesture(event, 'end')}
          onkeydown={(event) => handleBoundKeydown(event, 'end')}
        ></div>
      </div>
    {/if}
  </div>

  <div class="timeline-footer">
    <div class="timeline-status" role="status" aria-live="polite">{statusText}</div>
    <div class="timeline-controls">
      <button
        type="button"
        class="timeline-control timeline-zoom-out"
        aria-label={$t('ui.zoom_out', { default: 'Zoom Out' })}
        onclick={zoomOut}
      >
        <Icon name="minus" width={16} height={16} />
      </button>
      <button
        type="button"
        class="timeline-control timeline-zoom-in"
        aria-label={$t('ui.zoom_in', { default: 'Zoom In' })}
        onclick={zoomIn}
      >
        <Icon name="plus" width={16} height={16} />
      </button>
      <button
        type="button"
        class="timeline-control timeline-fit-all"
        aria-label={$t('ui.timeline_fit_all', { default: 'Fit all' })}
        onclick={fitAll}
      >
        <Icon name="maximize" width={16} height={16} />
      </button>
    </div>
  </div>
</div>

<style>
  .timeline-selector {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    width: 100%;
  }

  .timeline-ruler {
    position: relative;
    height: var(--space-5);
    cursor: grab;
    touch-action: none;
  }

  .timeline-ruler-label {
    position: absolute;
    top: 0;
    font-size: var(--font-sm);
    color: var(--text-secondary);
    white-space: nowrap;
    letter-spacing: normal;
  }

  .timeline-lane {
    position: relative;
    height: 56px;
    display: block;
    overflow: hidden;
    touch-action: none;
  }

  /* Full lane height, so a period with no photos is still a pointer target:
     FR-007 lets a user *attempt* the activation of an empty period, which a
     zero-height button (bar height 0) could not do. The bar stays the visible
     density profile. */
  .timeline-column {
    position: absolute;
    top: 0;
    bottom: 0;
    border: 0;
    background: transparent;
    padding: 0;
    display: flex;
    align-items: flex-end;
    justify-content: center;
    cursor: pointer;
  }

  .timeline-column-bar {
    width: 100%;
    border-radius: var(--radius-sm) var(--radius-sm) 0 0;
    background: color-mix(in oklch, var(--primary-color) 22%, transparent);
  }

  .timeline-column.empty .timeline-column-bar {
    background: var(--background-secondary);
  }

  /* Hover/focus reveals the period (FR-006) with a lighter tint than the
     active state, which stays the strongest signal on the lane. */
  .timeline-column.hovered .timeline-column-bar {
    background: color-mix(in oklch, var(--primary-color) 30%, transparent);
  }

  .timeline-column.active .timeline-column-bar {
    background: color-mix(in oklch, var(--primary-color) 45%, transparent);
  }

  .timeline-column:focus-visible {
    outline: none;
    box-shadow:
      0 0 0 2px var(--surface-color),
      0 0 0 4px var(--primary-color);
  }

  .timeline-selection {
    position: absolute;
    bottom: 0;
    top: 0;
    background: color-mix(in oklch, var(--primary-color) 14%, transparent);
    border: 1px solid var(--primary-color);
    border-radius: var(--radius-sm);
    pointer-events: none;
  }

  /* The grab zone has to clear the axe `target-size` floor (WCAG 2.5.8, 24px:
     a 12px slider is a violation once the handles are in the scanned tree).
     The handle paints nothing of its own — the selection's border is the
     visible edge — so widening the hit area costs no visual change and the
     drag stays exact (a press on it moves its own bound, never brushes). */
  .timeline-handle {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 24px;
    pointer-events: auto;
    cursor: ew-resize;
  }

  .timeline-handle.start {
    left: 0;
  }

  .timeline-handle.end {
    right: 0;
  }

  /* The ring is what makes a focused bound visible (SC-004/FR-014). */
  .timeline-handle:focus-visible {
    outline: none;
    box-shadow:
      0 0 0 2px var(--surface-color),
      0 0 0 4px var(--primary-color);
  }

  .timeline-footer {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }

  .timeline-status {
    flex: 1;
    font-size: var(--font-xs);
    color: var(--text-secondary);
    min-height: var(--space-5);
  }

  .timeline-controls {
    display: flex;
    gap: var(--space-2);
    margin-left: auto;
  }

  /* Measurement helper: it must keep the ruler labels' font, but a laid-out
     box would otherwise show up in the label geometry (its static position is
     the selector's content edge, on top of the first label). */
  .timeline-label-probe {
    position: absolute;
    visibility: hidden;
    pointer-events: none;
    width: 0;
    overflow: hidden;
  }

  .timeline-control {
    width: var(--space-8);
    height: var(--space-8);
    display: flex;
    align-items: center;
    justify-content: center;
    border: 1px solid var(--divider-color);
    border-radius: var(--radius-full);
    background: transparent;
    color: var(--text-secondary);
    cursor: pointer;
    transition:
      border-color var(--transition-fast),
      color var(--transition-fast),
      background-color var(--transition-fast);
  }

  .timeline-control:hover {
    border-color: var(--primary-color);
    color: var(--primary-color);
    background: color-mix(in oklch, var(--primary-color) 10%, transparent);
  }

  .timeline-control:focus-visible {
    outline: none;
    box-shadow:
      0 0 0 2px var(--surface-color),
      0 0 0 4px var(--primary-color);
  }

  .timeline-selector :global(svg) {
    width: 16px;
    height: 16px;
  }

  @media (prefers-reduced-motion: reduce) {
    .timeline-control {
      transition: none;
    }
  }
</style>

<script>
  // Date scrollbar controller: library density → filtered month model → a
  // bounded, bidirectionally growing listing window. The rail is navigation
  // only; the timeline lane remains the filter.
  import { onMount, tick } from 'svelte';
  import { MediaQuery } from 'svelte/reactivity';
  import { locale } from 'svelte-i18n';
  import { t } from '../lib/i18n.js';
  import { api } from '../lib/api.js';
  import { photoGridState, addToast } from '../lib/state.svelte.js';
  import { route } from '../lib/router.svelte.js';
  import { APP_CONSTANTS } from '../lib/constants.js';
  import { routeToPhotoQuery } from '../lib/photoQuery.js';
  import { normalizeDateFilter } from '../lib/timelineRoute.js';
  import { buildTimelineModel, formatPeriodName } from '../lib/timeline.js';
  import {
    anchorOffset,
    bottomTrim,
    earlierPage,
    monthOfPhoto,
    snapMonth,
    sliceDensity,
  } from '../lib/dateScrubber.js';
  import DateScrubberRail from './DateScrubberRail.svelte';

  const BATCH = APP_CONSTANTS.DEFAULT_BATCH_SIZE;
  const WINDOW_MAX_ITEMS = BATCH * 6;
  const EARLIER_MIN_VISIBLE_MS = 350;
  const EARLIER_ROOT_MARGIN = '600px 0px 0px 0px';

  const desktop = new MediaQuery(`(min-width: ${APP_CONSTANTS.MOBILE_BREAKPOINT + 1}px)`);
  const activeLocale = $derived($locale || 'en');

  const monthName = (month) => {
    const monthKey = APP_CONSTANTS.MONTH_KEYS[month - 1];
    return $t(`ui.months.${monthKey}`, { locale: activeLocale, default: monthKey });
  };

  let density = $state(null);
  let densityLoading = false;
  let visibleMonth = $state(null);
  let targetMonth = $state(null);
  let scrollEl = $state(null);
  let sentinelEl = $state(null);
  let earlierBusy = $state(false);
  let contentCenter = $state(0);
  let pendingJump = $state(false);
  let failedMonth = $state(null);

  // Bounded window bookkeeping.
  let startOffset = 0;
  let windowActive = false;
  let lastScope = '';
  let upLoading = false;
  let upGen = 0;
  let trimming = false;
  let windowBusy = false;
  let observedCount = 0;
  let rafId = 0;
  let suppressUntil = 0;
  let stickyTimer = null;
  let earlierShownAt = 0;
  let earlierHideTimer = null;

  const filter = $derived(
    normalizeDateFilter({
      year: route.year,
      month: route.month,
      to_year: route.to_year,
      to_month: route.to_month,
    })
  );
  const dateFiltered = $derived(filter.year !== null);
  const canScrub = $derived(
    route.view === 'all' &&
      route.query === null &&
      route.album === null &&
      !photoGridState.semanticSearchMode &&
      (route.sort === 'date_desc' || route.sort === 'date_asc')
  );
  const enabled = $derived(canScrub && desktop.current);
  const descending = $derived(route.sort !== 'date_asc');
  const photoCount = $derived(photoGridState.photos.length);
  const filteredDensity = $derived(
    density === null ? [] : sliceDensity(density.density ?? [], filter)
  );
  const model = $derived(buildTimelineModel(filteredDensity));
  const populatedMonths = $derived(
    model.length === 0 ? 0 : model.counts.reduce((count, value) => count + (value > 0 ? 1 : 0), 0)
  );
  const showRail = $derived(
    enabled &&
      model.length > 0 &&
      (dateFiltered ? populatedMonths >= 2 && model.total > BATCH : model.years.length > 2)
  );
  const activeMonth = $derived.by(() => {
    if (model.length === 0) return null;
    const fallback = descending ? model.maxIndex : model.minIndex;
    return snapMonth(model, targetMonth ?? visibleMonth ?? fallback) ?? fallback;
  });
  const scopeKey = $derived(
    `${route.view}:${route.query ?? ''}:${route.album ?? 0}:${route.sort}:${filter.year}:${filter.month}:${filter.to_year}:${filter.to_month}`
  );

  const headerLine = () =>
    Number.parseFloat(
      getComputedStyle(document.documentElement).getPropertyValue('--header-height')
    ) || 64;

  /** Center of the visible grid (the rail floats over its right edge). */
  function updateContentCenter() {
    if (!scrollEl) return;
    const target = document.getElementById('photo-grid') ?? scrollEl;
    const rect = target.getBoundingClientRect();
    contentCenter = rect.left + rect.width / 2;
  }

  /** Transform-free document position; card entrance animations offset rects. */
  function documentTop(el) {
    let top = 0;
    let node = el;
    while (node) {
      top += node.offsetTop;
      node = node.offsetParent;
    }
    return top;
  }

  /**
   * Keeps a card at its current viewport position after a window mutation.
   * `reference.documentTop` is the layout position captured before the
   * mutation; `place` puts the card under the fixed header instead.
   */
  function pinReference(reference, { place = false } = {}) {
    if (!scrollEl || !reference?.id) return;
    const el = document.querySelector(`#photo-grid [data-photo-id="${reference.id}"]`);
    if (!el) return;
    const newTop = documentTop(el);
    if (place) {
      scrollEl.scrollTop = newTop - documentTop(scrollEl) - headerLine();
    } else if (reference.documentTop !== undefined) {
      scrollEl.scrollTop += newTop - reference.documentTop;
    }
  }

  /** Adds the transform-free position a pin needs to restore after a mutation. */
  function withDocumentTop(reference) {
    if (!reference?.id) return reference;
    const el = document.querySelector(`#photo-grid [data-photo-id="${reference.id}"]`);
    return el ? { ...reference, documentTop: documentTop(el) } : reference;
  }

  /** The first card the user can see, by DOM order. */
  function firstVisibleCard() {
    if (!scrollEl) return null;
    const top = headerLine();
    const cards = document.querySelectorAll('#photo-grid .photo-card');
    for (let index = 0; index < cards.length; index += 1) {
      const rect = cards[index].getBoundingClientRect();
      if (rect.bottom > top + 1) return { index, id: cards[index].dataset.photoId };
    }
    return null;
  }

  function measureVisibleMonth() {
    if (!scrollEl) return;
    const list = photoGridState.photos;
    if (list.length === 0) {
      visibleMonth = null;
      return;
    }
    const visible = firstVisibleCard();
    visibleMonth =
      monthOfPhoto(list[visible?.index ?? list.length - 1]) ?? monthOfPhoto(list[list.length - 1]);
  }

  function scheduleMeasure() {
    if (rafId) return;
    rafId = requestAnimationFrame(() => {
      rafId = 0;
      measureVisibleMonth();
    });
  }

  function showEarlier() {
    if (earlierHideTimer !== null) {
      clearTimeout(earlierHideTimer);
      earlierHideTimer = null;
    }
    earlierShownAt = performance.now();
    earlierBusy = true;
  }

  function hideEarlier() {
    const remaining = Math.max(0, EARLIER_MIN_VISIBLE_MS - (performance.now() - earlierShownAt));
    if (earlierHideTimer !== null) clearTimeout(earlierHideTimer);
    earlierHideTimer = setTimeout(() => {
      earlierHideTimer = null;
      earlierBusy = false;
    }, remaining);
  }

  async function fetchDensity() {
    if (densityLoading) return;
    densityLoading = true;
    try {
      density = await api.request('/api/photos/timeline');
    } catch (error) {
      console.warn('[date-scrubber] timeline density unavailable', error);
      addToast(
        $t('notifications.error', { default: 'Error' }),
        $t('errors.timeline_load_failed', { default: 'Failed to load timeline data' }),
        'error',
        4000
      );
    } finally {
      densityLoading = false;
    }
  }

  /**
   * The page that holds the items immediately before the window, sliced to the
   * prefix that precedes `startOffset`. Exact even when the window starts
   * mid-page (the anchor page's dropped prefix).
   */
  async function loadEarlier({ place = false } = {}) {
    // `windowBusy` covers the anchor jump: it has already replaced the listing
    // and moved `startOffset`, so a wheel-up mid-jump must not fetch the page
    // before the new offset and prepend it to the old window.
    if (upLoading || windowBusy || !scrollEl || !canScrub || startOffset <= 0) return;
    const step = earlierPage(startOffset, BATCH);
    if (step === null) return;
    upLoading = true;
    setWindowBusy(true);
    showEarlier();
    const gen = upGen;
    try {
      const response = await api.getPhotos({
        page: step.page,
        limit: BATCH,
        ...routeToPhotoQuery(route, { view: 'grid' }),
      });
      if (gen !== upGen) return;
      const prepend = (response.photos ?? []).slice(0, step.take);
      if (prepend.length === 0) return;
      // The page header/timeline above the grid is the browser's anchor node at
      // the window top, so native scroll anchoring leaves the reading card
      // behind. Pin the card explicitly; the jump additionally places it at
      // the top edge.
      const reference = withDocumentTop(firstVisibleCard());
      photoGridState.photos = [...prepend, ...photoGridState.photos];
      startOffset -= prepend.length;
      await tick();
      if (!scrollEl || gen !== upGen) return;
      pinReference(reference, { place });
      suppressUntil = performance.now() + 250;
      scheduleMeasure();
    } catch (error) {
      console.warn('[date-scrubber] earlier page unavailable', error);
      addToast(
        $t('notifications.error', { default: 'Error' }),
        $t('errors.error_loading_photos', { default: 'Error Loading Photos' }),
        'error',
        5000
      );
    } finally {
      upLoading = false;
      // A jump or reset that superseded this fetch owns the busy flag now; the
      // superseded fetch must not release its lock.
      if (gen === upGen) setWindowBusy(false);
      hideEarlier();
      void balanceWindow();
    }
  }

  /**
   * Keeps the window at WINDOW_MAX_ITEMS cards, trimming the side away from the
   * viewport. Dropping above pins the visible card; dropping below re-arms the
   * grid's own downward loader for the dropped page.
   */
  async function balanceWindow() {
    if (!enabled || !windowActive || trimming || windowBusy || !scrollEl) return;
    if (photoGridState.photos.length <= WINDOW_MAX_ITEMS) return;
    trimming = true;
    try {
      let guard = 0;
      while (photoGridState.photos.length > WINDOW_MAX_ITEMS && guard < 8) {
        guard += 1;
        const visible = firstVisibleCard();
        const length = photoGridState.photos.length;
        if (visible && visible.index > length / 2) await trimTop();
        else await trimBottom();
        await tick();
      }
    } finally {
      trimming = false;
      scheduleMeasure();
    }
  }

  async function trimTop() {
    const photos = photoGridState.photos;
    const drop = photos.length - WINDOW_MAX_ITEMS;
    if (drop <= 0) return;
    const reference = withDocumentTop(firstVisibleCard());
    photoGridState.photos = photos.slice(drop);
    startOffset += drop;
    await tick();
    pinReference(reference);
  }

  async function trimBottom() {
    const trim = bottomTrim(startOffset, photoGridState.photos.length, BATCH);
    if (trim === null) return;
    photoGridState.photos = photoGridState.photos.slice(0, -trim.drop);
    // The grid's own loader must re-fetch the page that was just dropped.
    photoGridState.currentPage = trim.page;
    photoGridState.hasMore = true;
    photoGridState.windowEpoch += 1;
    await tick();
  }

  async function anchorTo(index) {
    if (!scrollEl || !canScrub || model.length === 0) return false;
    const offset = anchorOffset(model, index, {
      descending,
      totalPhotos: photoGridState.total,
    });
    const page = Math.floor(offset / BATCH) + 1;
    const skip = offset - (page - 1) * BATCH;
    const gen = upGen + 1;
    upGen = gen;
    setWindowBusy(true);
    try {
      const response = await api.getPhotos({
        page,
        limit: BATCH,
        ...routeToPhotoQuery(route, { view: 'grid' }),
      });
      // A newer jump or a scope change owns the listing now; applying this
      // response would leave the grid on the old scope's anchored window and
      // make `currentPage` point into the middle of the new listing.
      if (gen !== upGen) return false;
      const items = (response.photos ?? []).slice(skip);
      if (items.length === 0) return false;
      // Only a loaded window may move `startOffset`: a failed jump leaves the
      // existing window (and its offset) untouched.
      startOffset = offset;
      photoGridState.photos = items;
      // `currentPage` is the NEXT page the grid will fetch (appendPhotos
      // increments it after a load), so the anchor's own page must not repeat.
      photoGridState.currentPage = page + 1;
      photoGridState.hasMore = Boolean(response.has_next);
      windowActive = true;
      photoGridState.windowEpoch += 1;
      suppressUntil = performance.now() + 400;
      scrollEl.scrollTop = 0;
      // The preload captures the first visible card; let the anchor's cards
      // render first so that reference exists.
      await tick();
      return true;
    } catch (error) {
      console.warn('[date-scrubber] anchor page unavailable', error);
      addToast(
        $t('notifications.error', { default: 'Error' }),
        $t('errors.date_jump_failed', { default: 'Could not load that period' }),
        'error',
        4000
      );
      return false;
    } finally {
      // A superseded call must not clear the busy flag of the jump that
      // replaced it.
      if (gen === upGen) setWindowBusy(false);
    }
  }

  async function settleOn(index) {
    const next = snapMonth(model, index);
    if (next === null) return;
    targetMonth = next;
    pendingJump = true;
    failedMonth = null;
    const anchored = await anchorTo(next);
    pendingJump = false;
    if (!anchored) {
      failedMonth = next;
      targetMonth = null;
      measureVisibleMonth();
      return;
    }
    // Content above the target plus the target itself at the top edge.
    void loadEarlier({ place: true });
    addToast(
      $t('notifications.success', { default: 'Success' }),
      $t('notifications.date_jump_done', {
        locale: activeLocale,
        values: { period: formatPeriodName(next, monthName) },
        default: 'Jumped to {period}',
      }),
      'success',
      3000
    );
    if (stickyTimer !== null) clearTimeout(stickyTimer);
    stickyTimer = setTimeout(() => {
      stickyTimer = null;
      targetMonth = null;
      measureVisibleMonth();
    }, 1400);
  }

  const scrubTo = (index) => {
    const next = snapMonth(model, index);
    if (next !== null) targetMonth = next;
  };

  const endScrub = () => {
    if (targetMonth !== null) void settleOn(targetMonth);
  };

  const cancelScrub = () => {
    targetMonth = null;
    pendingJump = false;
    measureVisibleMonth();
  };

  function onScroll() {
    if (performance.now() < suppressUntil) return;
    targetMonth = null;
    scheduleMeasure();
  }

  const onWheel = (event) => {
    if (event.deltaY < 0 && scrollEl && scrollEl.scrollTop <= 0) void loadEarlier();
  };

  /** Mirrors the window lock into the grid so assistive tech sees the jump. */
  function setWindowBusy(value) {
    windowBusy = value;
    photoGridState.windowBusy = value;
  }

  /**
   * Forgets the anchored window: a listing reload replaces the photos with
   * page 1 while the scope key stays the same, so the stale offset would
   * prepend a middle-of-library page on top of it.
   */
  function resetWindow() {
    upGen += 1;
    startOffset = 0;
    windowActive = false;
    // A reload abandons the window unconditionally; a fetch it superseded must
    // not leave the lock latched now that it can no longer release it itself.
    setWindowBusy(false);
    targetMonth = null;
    pendingJump = false;
    failedMonth = null;
  }

  function handleReloadRequest() {
    // The grid reloads page 1 regardless of the viewport, so the window must
    // be forgotten even when the rail is dormant (`enabled` folds in the
    // media query, which `scopeKey` does not cover).
    resetWindow();
    void fetchDensity();
  }

  $effect(() => {
    if (enabled && density === null) void fetchDensity();
  });

  $effect(() => {
    if (!scrollEl) return;
    scrollEl.classList.toggle('date-scrubber-active', showRail);
    // Reserve the lane's edge for the view header only: the glass floats over
    // the grid, but Select/Sort must stay clickable at the content's edge.
    scrollEl.style.setProperty('--date-rail-inset', showRail ? '42px' : '');
  });

  $effect(() => {
    const nextScope = scopeKey;
    if (nextScope === lastScope) return;
    lastScope = nextScope;
    resetWindow();
    scheduleMeasure();
  });

  $effect(() => {
    const count = photoCount;
    if (count === observedCount) return;
    observedCount = count;
    void balanceWindow();
  });

  $effect(() => {
    if (!scrollEl || !sentinelEl || !enabled) return;
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) void loadEarlier();
      },
      { root: scrollEl, rootMargin: EARLIER_ROOT_MARGIN }
    );
    observer.observe(sentinelEl);
    return () => observer.disconnect();
  });

  onMount(() => {
    const scrollTarget = document.querySelector('.main-content');
    scrollEl = scrollTarget;
    updateContentCenter();
    scrollTarget?.addEventListener('scroll', onScroll, { passive: true });
    scrollTarget?.addEventListener('wheel', onWheel, { passive: true });
    window.addEventListener('photosReloadRequested', handleReloadRequest);
    window.addEventListener('indexingCompleted', handleReloadRequest);

    const onResize = () => {
      updateContentCenter();
      scheduleMeasure();
    };
    window.addEventListener('resize', onResize);
    const observer = new ResizeObserver(() => {
      updateContentCenter();
      scheduleMeasure();
    });
    if (scrollTarget) observer.observe(scrollTarget);
    const grid = document.getElementById('photo-grid');
    if (grid) observer.observe(grid);

    return () => {
      // A teardown must invalidate an in-flight anchor jump: its continuation
      // would otherwise write the old listing after a remounted grid started
      // loading page 1.
      resetWindow();
      if (rafId) cancelAnimationFrame(rafId);
      if (stickyTimer !== null) clearTimeout(stickyTimer);
      if (earlierHideTimer !== null) clearTimeout(earlierHideTimer);
      scrollTarget?.removeEventListener('scroll', onScroll);
      scrollTarget?.removeEventListener('wheel', onWheel);
      window.removeEventListener('resize', onResize);
      window.removeEventListener('photosReloadRequested', handleReloadRequest);
      window.removeEventListener('indexingCompleted', handleReloadRequest);
      observer.disconnect();
      scrollTarget?.classList.remove('date-scrubber-active');
      scrollTarget?.style.removeProperty('--date-rail-inset');
    };
  });
</script>

<div class="date-scrubber-sentinel" bind:this={sentinelEl} aria-hidden="true"></div>

{#if showRail}
  <DateScrubberRail
    {model}
    {descending}
    {activeMonth}
    pending={pendingJump}
    {failedMonth}
    onScrub={scrubTo}
    onScrubEnd={endScrub}
    onCancel={cancelScrub}
    onJump={settleOn}
  />
{/if}

{#if earlierBusy}
  <div
    class="earlier-loading"
    data-testid="date-earlier-loading"
    style:left={`${contentCenter}px`}
    role="status"
    aria-live="polite"
  >
    <span class="dot-wave" aria-hidden="true">
      <span class="dot-wave-dot"></span>
      <span class="dot-wave-dot"></span>
      <span class="dot-wave-dot"></span>
    </span>
    <span class="sr-only"
      >{$t('ui.loading_photos', { locale: activeLocale, default: 'Loading photos…' })}</span
    >
  </div>
{/if}

<style>
  .date-scrubber-sentinel {
    height: 0;
  }

  /* The rail replaces the native bar while it is shown and reserves its
     paper margin plus the caption's 10px overhang (42px = 56px lane − 24px
     main padding + 10px) for the view header, so Select/Sort stay clickable
     clear of the caption's box. The banner/header is the browser's scroll
     anchor at the window top, so card pinning is explicit and native anchoring
     is deliberately opted out. */
  :global(.main-content.date-scrubber-active) {
    scrollbar-width: none;
    overflow-anchor: none;
  }

  :global(.main-content.date-scrubber-active)::-webkit-scrollbar {
    width: 0;
    height: 0;
  }

  .earlier-loading {
    position: fixed;
    top: calc(var(--header-height) + var(--space-3));
    left: 0;
    z-index: var(--z-sticky);
    display: flex;
    align-items: center;
    justify-content: center;
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--glass-border);
    border-radius: var(--radius-full);
    background: var(--surface-color);
    background: var(--glass-bg);
    box-shadow: var(--shadow-medium);
    pointer-events: none;
    transform: translateX(-50%);
    backdrop-filter: blur(16px) saturate(1.5);
  }

  .dot-wave {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .dot-wave-dot {
    width: var(--space-3);
    height: var(--space-3);
    border-radius: var(--radius-full);
    background-color: var(--primary-color);
    box-shadow: 0 2px 4px rgb(0 0 0 / 10%);
    animation: dot-wave 1.4s ease-in-out infinite;
  }

  .dot-wave-dot:nth-child(2) {
    animation-delay: 0.2s;
  }

  .dot-wave-dot:nth-child(3) {
    animation-delay: 0.4s;
  }

  @keyframes dot-wave {
    0%,
    60%,
    100% {
      opacity: 0.7;
      transform: translateY(0);
    }

    30% {
      opacity: 1;
      transform: translateY(-15px);
    }
  }

  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    margin: -1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
    border: 0;
  }

  @media (prefers-reduced-motion: reduce) {
    .dot-wave-dot {
      animation: none;
    }
  }
</style>

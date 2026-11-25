<script>
  import { onDestroy, onMount } from 'svelte';

  import { firstVisibleCardIndex } from '../lib/dateScrubber.js';
  import {
    formatHintDate,
    isDateSort,
    isPhotoGridRoute,
    nextHintVisibility,
  } from '../lib/dateHint.js';
  import { getLocale, t } from '../lib/i18n.js';
  import { route } from '../lib/router.svelte.js';
  import { appState, photoGridState } from '../lib/state.svelte.js';

  /**
   * Current-date hint: the date of the topmost visible photo while the view
   * header (title, Select, Sort) is under the fixed glass bar. It is a fixed
   * layer whose left edge follows the content area (sidebar, collapse, mobile)
   * and whose right edge honours the date rail's inset; no scroll geometry is
   * involved.
   */

  const HYSTERESIS = 8;

  let minimized = $state(false);
  let label = $state('');
  let gridPresent = $state(false);
  let scrollEl = null;
  let rafId = 0;
  let lastTakenAt;
  let lastLocale;

  const chronological = $derived(
    isPhotoGridRoute(route) && isDateSort(route.sort) && !photoGridState.semanticSearchMode
  );
  const visible = $derived(
    minimized &&
      chronological &&
      gridPresent &&
      label !== '' &&
      !appState.dateRailBubbleVisible &&
      !appState.dateRailLoading &&
      !appState.sidebarOpen &&
      !appState.mobileSearchOpen
  );
  const routeKey = $derived(
    [
      route.view,
      route.query ?? '',
      route.album ?? 0,
      route.person ?? 0,
      route.sort,
      route.year ?? '',
      route.month ?? '',
      route.to_year ?? '',
      route.to_month ?? '',
    ].join(':')
  );
  const scopeKey = $derived(`${routeKey}:${photoGridState.photos.length}`);

  function headerLine() {
    return (
      Number.parseFloat(
        getComputedStyle(document.documentElement).getPropertyValue('--header-height')
      ) || 64
    );
  }

  function measure() {
    rafId = 0;
    const header = headerLine();
    const contentHeader = document.querySelector('.content-header');
    const grid = document.getElementById('photo-grid');
    gridPresent = Boolean(grid?.querySelector('.photo-card'));

    if (!contentHeader || !grid) {
      minimized = false;
      label = '';
      lastTakenAt = undefined;
      return;
    }

    minimized = nextHintVisibility(
      contentHeader.getBoundingClientRect().bottom,
      minimized,
      header,
      HYSTERESIS
    );

    const index = firstVisibleCardIndex(
      grid.querySelectorAll('.photo-card'),
      header,
      window.innerHeight
    );
    const photo = index === null ? null : photoGridState.photos[index];
    if (!photo) {
      label = '';
      lastTakenAt = undefined;
      return;
    }

    const takenAt = photo.taken_at ?? null;
    const locale = getLocale();
    if (takenAt !== lastTakenAt || locale !== lastLocale) {
      lastTakenAt = takenAt;
      lastLocale = locale;
      label = formatHintDate(takenAt, locale) ?? $t('ui.no_date');
    }
  }

  function scheduleMeasure() {
    if (rafId) return;
    rafId = requestAnimationFrame(measure);
  }

  // A route change drops the old view's date immediately; the grid may already
  // be gone while the next rAF has not measured its replacement.
  $effect(() => {
    if (routeKey) {
      minimized = false;
      label = '';
      gridPresent = false;
      lastTakenAt = undefined;
      scheduleMeasure();
    }
  });

  // Listing changes (append/prepend/filter reload) re-measure without hiding.
  $effect(() => {
    if (scopeKey) scheduleMeasure();
  });

  $effect(() => {
    appState.dateHintVisible = visible;
  });

  onMount(() => {
    scrollEl = document.querySelector('.main-content');
    scrollEl?.addEventListener('scroll', scheduleMeasure, { passive: true });
    window.addEventListener('resize', scheduleMeasure);
    scheduleMeasure();

    return () => {
      scrollEl?.removeEventListener('scroll', scheduleMeasure);
      window.removeEventListener('resize', scheduleMeasure);
      if (rafId) cancelAnimationFrame(rafId);
    };
  });

  onDestroy(() => {
    appState.dateHintVisible = false;
  });
</script>

<div
  class="date-hint-layer"
  class:visible
  class:sidebar-collapsed={appState.sidebarCollapsed}
  data-testid="date-hint"
  data-visible={visible ? 'true' : 'false'}
  aria-hidden="true"
>
  <span class="date-hint" data-testid="date-hint-pill">{label}</span>
</div>

<style>
  .date-hint-layer {
    position: fixed;
    top: calc(var(--header-height) + var(--space-2));
    right: var(--date-rail-inset, 0px);
    left: var(--sidebar-width);
    z-index: var(--z-dropdown);
    display: flex;
    justify-content: center;
    opacity: 0;
    pointer-events: none;
    transform: translateY(-4px);
    transition:
      opacity var(--transition-medium),
      transform var(--transition-medium);
  }

  .date-hint-layer.visible {
    opacity: 1;
    transform: none;
    transition:
      opacity var(--transition-fast),
      transform var(--transition-fast);
  }

  @media (width <= 768px) {
    .date-hint-layer {
      left: 0;
    }
  }

  @media (width > 768px) {
    .date-hint-layer.sidebar-collapsed {
      left: 0;
    }
  }

  .date-hint {
    display: inline-flex;
    align-self: flex-start;
    align-items: center;
    padding: 6px var(--space-4);
    border: 1px solid var(--glass-border, var(--divider-color));
    border-radius: var(--radius-full);
    background: var(--glass-bg);
    backdrop-filter: blur(16px) saturate(1.5);
    -webkit-backdrop-filter: blur(16px) saturate(1.5);
    box-shadow: var(--shadow-medium);
    color: var(--text-primary);
    font-family: var(--font-body);
    font-size: var(--font-sm);
    font-weight: var(--font-medium);
    line-height: 1.4;
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  @supports not (backdrop-filter: blur(1px)) {
    .date-hint {
      background: var(--surface-color);
    }
  }

  @media (prefers-reduced-transparency: reduce) {
    .date-hint {
      background: var(--surface-color);
      backdrop-filter: none;
      -webkit-backdrop-filter: none;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .date-hint-layer,
    .date-hint-layer.visible {
      transition: none;
    }
  }
</style>

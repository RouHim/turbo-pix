<script>
  import { onMount, onDestroy } from 'svelte';
  import { t } from '../lib/i18n.js';
  import Icon from '../lib/Icon.svelte';
  import { api } from '../lib/api.js';
  import { indexingState } from '../lib/state.svelte.js';
  import { logger } from '../lib/logger.js';

  const POLL_FREQ_INDEXING = 1000;
  const POLL_FREQ_IDLE = 30000;
  const HIDE_DELAY = 2000;
  const STORAGE_KEY = 'turbopix_has_indexed';

  const PHASES = [
    { id: 'discovering', icon: 'camera' },
    { id: 'metadata', icon: 'file-text' },
    { id: 'semantic_vectors', icon: 'cpu' },
    { id: 'geo_resolution', icon: 'map-pin' },
    { id: 'collages', icon: 'grid' },
    { id: 'housekeeping', icon: 'check-circle' },
  ];

  let pollTimer = $state(null);
  let hideTimer = $state(null);
  let autoOpened = $state(false);
  let completionPulse = $state(false);
  let destroyed = false;
  let sheetOpen = $state(false);
  let sheetCloseButton = $state(null);
  let ringTrigger = $state(null);
  let sheetHasOpened = false;
  // Only user-initiated opens may yank focus into the sheet; the auto-open
  // path (checkStatus → openSheet(true)) must not steal focus from the page.
  let sheetOpenedByUser = false;

  // Move focus into the bottom sheet when it opens and back to the ring on close.
  // The ring is only refocused while interactive — never when data-ring-mode='hidden'.
  $effect(() => {
    if (sheetOpen) {
      if (sheetOpenedByUser) {
        sheetHasOpened = true;
        sheetCloseButton?.focus();
      }
    } else if (sheetHasOpened) {
      sheetHasOpened = false;
      sheetOpenedByUser = false;
      if (ringMode !== 'hidden') {
        ringTrigger?.focus();
      }
    }
  });

  const ringMode = $derived(determineMode());

  function polarToCartesian(centerX, centerY, radius, angleInDegrees) {
    const angleInRadians = ((angleInDegrees - 90) * Math.PI) / 180;
    return {
      x: centerX + radius * Math.cos(angleInRadians),
      y: centerY + radius * Math.sin(angleInRadians),
    };
  }

  function describeArc(centerX, centerY, radius, index) {
    const gapDegrees = 4;
    const segmentDegrees = 60;
    const startAngle = -90 + index * segmentDegrees + gapDegrees / 2;
    const endAngle = startAngle + segmentDegrees - gapDegrees;
    const start = polarToCartesian(centerX, centerY, radius, startAngle);
    const end = polarToCartesian(centerX, centerY, radius, endAngle);
    return ['M', start.x, start.y, 'A', radius, radius, 0, 0, 1, end.x, end.y].join(' ');
  }

  function arcPaths() {
    return PHASES.map((_, i) => describeArc(140, 140, 120, i));
  }

  function normalizeStatus(status) {
    const phase = status.phase ?? status.active_phase_id ?? '';
    return {
      ...status,
      phase,
      active_phase_id: status.active_phase_id ?? phase,
      phases: Array.isArray(status.phases) ? status.phases : [],
    };
  }

  function hasIndexedBefore() {
    try {
      return localStorage.getItem(STORAGE_KEY) === 'true';
    } catch {
      return false;
    }
  }

  function prefersReducedMotion() {
    return window.matchMedia?.('(prefers-reduced-motion: reduce)')?.matches ?? false;
  }

  function determineMode() {
    if (completionPulse) return 'compact';
    if (!indexingState.isIndexing) return 'hidden';
    if (hasIndexedBefore()) return 'compact';
    const photosIndexed = Number(indexingState.photosIndexed ?? 0);
    if (photosIndexed === 0) return 'large';
    return 'compact';
  }

  function toggleSheet() {
    if (ringMode !== 'compact') return;
    if (sheetOpen) {
      closeSheet();
    } else {
      openSheet();
    }
  }

  function openSheet(force = false) {
    if (!force && ringMode !== 'compact') return;
    sheetOpen = true;
  }

  function closeSheet() {
    sheetOpen = false;
  }

  function markIndexingCompleted() {
    try {
      localStorage.setItem(STORAGE_KEY, 'true');
    } catch {
      // storage unavailable
    }
  }

  function cancelPendingHide() {
    if (hideTimer) {
      clearTimeout(hideTimer);
      hideTimer = null;
    }
    // A fresh indexing status (or an aborted hide) must end any in-flight
    // completion pulse; otherwise the ring/sheet stay stuck in the 'all done'
    // visual state for the whole of a new indexing run.
    completionPulse = false;
  }

  function hideRing({ showCompletionPulse = false } = {}) {
    cancelPendingHide();
    if (showCompletionPulse) {
      completionPulse = true;
      hideTimer = setTimeout(() => {
        completionPulse = false;
        closeSheet();
      }, HIDE_DELAY);
      return;
    }
    completionPulse = false;
    closeSheet();
  }

  const ARC_LENGTH = 125.66;

  function updateOrbit(status) {
    cancelPendingHide();

    indexingState.isIndexing = status.is_indexing;
    indexingState.currentPhase = status.phase;
    indexingState.phases = status.phases;
    indexingState.photosIndexed = status.photos_indexed ?? 0;
  }

  async function checkStatus() {
    try {
      const status = await api.getIndexingStatus();
      const normalizedStatus = normalizeStatus(status);
      const wasIndexing = indexingState.isIndexing;

      if (normalizedStatus.is_indexing) {
        indexingState.isIndexing = true;
        updateOrbit(normalizedStatus);
        if (!autoOpened && determineMode() === 'large') {
          autoOpened = true;
          requestAnimationFrame(() => openSheet(true));
        }
      } else if (wasIndexing) {
        markIndexingCompleted();
        indexingState.isIndexing = false;
        hideRing({ showCompletionPulse: true });
        window.dispatchEvent(new CustomEvent('indexingCompleted'));
      } else {
        indexingState.isIndexing = false;
        hideRing();
      }

      let currentItem = '';
      for (const phase of normalizedStatus.phases) {
        if (phase.state === 'active' && phase.current_item) {
          currentItem = phase.current_item;
          break;
        }
      }
      indexingState.currentItem = currentItem;

      window.dispatchEvent(
        new CustomEvent('indexingStatusChanged', {
          detail: normalizedStatus,
        })
      );
    } catch (error) {
      logger.error('Failed to check indexing status', error, {
        component: 'IndexingOrbit',
      });
    } finally {
      scheduleNextPoll();
    }
  }

  function scheduleNextPoll() {
    if (destroyed) return;
    if (pollTimer) clearTimeout(pollTimer);
    const freq = indexingState.isIndexing ? POLL_FREQ_INDEXING : POLL_FREQ_IDLE;
    pollTimer = setTimeout(() => checkStatus(), freq);
  }

  function startPolling() {
    checkStatus();
    logger.debug('Indexing status polling started', { component: 'IndexingOrbit' });
  }

  function stopPolling() {
    if (pollTimer) {
      clearTimeout(pollTimer);
      pollTimer = null;
      logger.debug('Indexing status polling stopped', { component: 'IndexingOrbit' });
    }
  }

  function onKeydown(e) {
    if (e.key === 'Escape') closeSheet();
  }

  onMount(() => {
    window.indexingStatus = {
      async checkStatus() {
        await checkStatus();
      },
      get isIndexing() {
        return indexingState.isIndexing;
      },
    };

    document.addEventListener('keydown', onKeydown);
    startPolling();
  });

  onDestroy(() => {
    destroyed = true;
    cancelPendingHide();
    stopPolling();
    document.removeEventListener('keydown', onKeydown);
    if (window.indexingStatus) {
      // Keep the object for E2E compatibility
      window.indexingStatus.checkStatus = async () => {};
    }
  });

  const sheetPhases = $derived(
    indexingState.phases.map((phase) => {
      const def = PHASES.find((p) => p.id === phase.id);
      const phaseName = $t(`ui.indexing_phase_${phase.id}`) || def?.id || phase.id;
      const isDeterminate = phase.kind === 'determinate';
      const total = phase.total || 0;
      const processed = phase.processed || 0;
      const percent = completionPulse
        ? 100
        : isDeterminate && total > 0
          ? Math.round((processed / total) * 100)
          : 0;
      const errorsText =
        phase.errors && phase.errors > 0
          ? $t('ui.indexing_sheet_errors', {
              values: { count: phase.errors },
              default: `${phase.errors} error(s)`,
            })
          : '';
      return {
        ...phase,
        icon: def?.icon || 'camera',
        phaseName,
        isDeterminate,
        total,
        processed,
        percent,
        errorsText,
        countLabel: isDeterminate ? `${processed}/${total}` : '—',
        isActive: completionPulse ? false : phase.state === 'active',
        isDone: completionPulse ? true : phase.state === 'done',
        isError: completionPulse ? false : phase.state === 'error',
      };
    })
  );

  const activePhaseName = $derived(
    (() => {
      for (const sp of sheetPhases) {
        if (sp.isActive) {
          const processed = sp.processed || 0;
          const total = sp.total || 0;
          if (total > 0) {
            const tpl = $t('ui.indexing_ring_tooltip', {
              values: { phase: sp.phaseName, processed, total },
              default: `${sp.phaseName} — ${processed}/${total}`,
            });
            return tpl;
          }
          // Indeterminate phase (no totals yet): a "— 0/0" tooltip would be
          // noise, show the phase name alone.
          return sp.phaseName;
        }
      }
      return '';
    })()
  );

  // Announced through the ring's aria-live region: screen-reader users get
  // progress updates even though the SVG and bottom sheet are aria-hidden.
  const liveStatusText = $derived(
    activePhaseName ||
      (completionPulse
        ? $t('ui.indexing_complete', { default: 'Indexing complete' })
        : $t('ui.indexing_photos', { default: 'Processing your photos...' }))
  );

  const tooltipText = $derived(ringMode === 'compact' && activePhaseName ? activePhaseName : '');

  const centerIcon = $derived(
    (() => {
      const activePhase = PHASES.find((p) => p.id === indexingState.currentPhase);
      return completionPulse ? 'check-circle' : activePhase?.icon || 'camera';
    })()
  );
</script>

<div
  data-phase-ring
  data-ring-mode={ringMode}
  class="indexing-orbit-ring"
  aria-live="polite"
  aria-label={$t('ui.indexing_photos', { default: 'Processing your photos...' })}
  aria-expanded={sheetOpen}
  title={tooltipText}
  onclick={() => {
    sheetOpenedByUser = true;
    toggleSheet();
  }}
  onkeydown={(e) => {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      sheetOpenedByUser = true;
      toggleSheet();
    }
  }}
  role="button"
  aria-hidden={ringMode === 'hidden'}
  tabindex={ringMode === 'hidden' || ringMode === 'large' ? -1 : 0}
  bind:this={ringTrigger}
>
  <div class="indexing-orbit-shell">
    <span class="sr-only">{liveStatusText}</span>
    <svg class="indexing-orbit-svg" viewBox="0 0 280 280" aria-hidden="true">
      {#each PHASES as phase, i (phase.id)}
        {@const arcD = arcPaths()[i]}
        {@const sp = sheetPhases.find((p) => p.id === phase.id)}
        {@const phaseState = completionPulse
          ? 'done'
          : sp?.isDone
            ? 'done'
            : sp?.isError
              ? 'error'
              : sp?.isActive
                ? 'active'
                : 'pending'}
        {@const dashOffset = completionPulse
          ? 0
          : sp?.isDone
            ? 0
            : sp?.isActive && sp.isDeterminate && sp.total > 0
              ? ARC_LENGTH * (1 - Math.min(Math.max(sp.processed / sp.total, 0), 1))
              : ARC_LENGTH}
        <path
          class="indexing-orbit-segment"
          d={arcD}
          data-phase-id={phase.id}
          data-phase-state={phaseState}
          stroke="currentColor"
          fill="none"
          style="stroke-dashoffset: {dashOffset}"
        ></path>
        {#if sp?.isActive && sp.isDeterminate === false}
          {@const midpointAngle = -90 + i * 60 + 30}
          {@const pos = polarToCartesian(140, 140, 120, midpointAngle)}
          <g
            data-orbit-phase={phase.id}
            style="transform-origin: 140px 140px; transform-box: view-box; animation: {prefersReducedMotion()
              ? 'none'
              : 'orbit-segment 2s ease-in-out infinite'}"
          >
            <circle cx={pos.x} cy={pos.y} class="orbit-dot" data-orbit-dot="true"></circle>
          </g>
        {/if}
      {/each}
    </svg>
    <div class="orbit-center-icon">
      <Icon name={centerIcon} width={32} height={32} />
    </div>
  </div>
</div>

<!-- Backdrop -->
<div
  class="indexing-sheet-backdrop"
  class:is-visible={sheetOpen}
  onclick={closeSheet}
  role="presentation"
></div>

<!-- Bottom Sheet -->
<div
  data-bottom-sheet
  class="indexing-bottom-sheet"
  role="dialog"
  aria-modal="true"
  aria-label={$t('ui.indexing_photos', { default: 'Processing your photos...' })}
  aria-hidden={!sheetOpen}
>
  <div class="indexing-sheet-handle"></div>
  <div class="indexing-sheet-header">
    <span class="indexing-sheet-title">
      {$t('ui.indexing_sheet_title', { default: 'Indexing Progress' })}
    </span>
    <button
      type="button"
      class="indexing-sheet-close"
      data-sheet-close
      aria-label={$t('ui.close', { default: 'Close' })}
      tabindex={sheetOpen ? 0 : -1}
      onclick={closeSheet}
      bind:this={sheetCloseButton}
    >
      <Icon name="x" width={20} height={20} />
    </button>
  </div>
  <div class="indexing-sheet-summary">
    <span data-sheet-photos-count>{indexingState.photosIndexed}</span>
    <span>{$t('ui.indexing_sheet_photos_indexed', { default: 'photos indexed' })}</span>
  </div>
  <div class="indexing-sheet-phases">
    {#each sheetPhases as phase (phase.id)}
      <div
        class="indexing-sheet-phase"
        class:is-active={phase.isActive}
        class:is-done={phase.isDone}
        class:is-error={phase.isError}
        data-phase-id={phase.id}
      >
        <Icon name={phase.icon} width={16} height={16} className="phase-icon" />
        <div class="phase-info">
          <span class="phase-name">{phase.phaseName}</span>
          <div class="phase-progress-bar">
            <div class="phase-progress-fill" data-phase-fill style="width: {phase.percent}%"></div>
          </div>
        </div>
        <span class="phase-count" data-phase-count>{phase.countLabel}</span>
        <span data-phase-errors class="indexing-phase-errors">{phase.errorsText}</span>
      </div>
    {/each}
  </div>
  <div class="indexing-sheet-current-item" data-sheet-current-item>
    {indexingState.currentItem}
  </div>
</div>

<style>
  /* Visually hidden but announced by the ring's aria-live region */
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    margin: -1px;
    padding: 0;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
    border: 0;
  }

  /* === Indexing Ring === */
  [data-phase-ring] {
    --indexing-ring-large-size: 300px;
    --indexing-ring-compact-size: 64px;
    --indexing-ring-compact-offset: var(--space-6);
    --orbit-center-icon-size: 32px;

    position: fixed;
    top: 50%;
    left: 50%;
    z-index: 900;
    width: var(--indexing-ring-large-size);
    height: var(--indexing-ring-large-size);
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--glass-bg);
    backdrop-filter: blur(16px) saturate(1.5);
    -webkit-backdrop-filter: blur(16px) saturate(1.5);
    border: 1px solid var(--glass-border);
    border-radius: 50%;
    box-shadow: var(--shadow-medium);
    opacity: 1;
    pointer-events: auto;
    transform: translate(-50%, -50%);
    transition:
      width 0.5s var(--ease-spring),
      height 0.5s var(--ease-spring),
      top 0.5s var(--ease-spring),
      left 0.5s var(--ease-spring),
      transform 0.5s var(--ease-spring),
      opacity var(--transition-medium);
  }

  [data-ring-mode='large'] {
    top: 50%;
    left: 50%;
    width: var(--indexing-ring-large-size);
    height: var(--indexing-ring-large-size);
    opacity: 1;
    transform: translate(-50%, -50%);
  }

  [data-ring-mode='compact'] {
    --orbit-center-icon-size: 16px;

    top: calc(100vh - var(--indexing-ring-compact-offset) - var(--indexing-ring-compact-size));
    left: calc(100vw - var(--indexing-ring-compact-offset) - var(--indexing-ring-compact-size));
    width: var(--indexing-ring-compact-size);
    height: var(--indexing-ring-compact-size);
    opacity: 1;
    transform: translate(0, 0);
    cursor: pointer;
  }

  [data-ring-mode='hidden'] {
    --orbit-center-icon-size: 16px;

    top: calc(100vh - var(--indexing-ring-compact-offset) - var(--indexing-ring-compact-size));
    left: calc(100vw - var(--indexing-ring-compact-offset) - var(--indexing-ring-compact-size));
    width: var(--indexing-ring-compact-size);
    height: var(--indexing-ring-compact-size);
    opacity: 0;
    pointer-events: none;
    transform: translate(0, 0) scale(0.8);
  }

  .indexing-orbit-shell {
    position: relative;
    width: 100%;
    height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .indexing-orbit-svg {
    width: 100%;
    height: 100%;
    overflow: visible;
  }

  .indexing-orbit-segment {
    color: var(--divider-color);
    stroke-width: 16;
    stroke-linecap: round;
    stroke-dasharray: 125.66;
    stroke-dashoffset: 125.66;
    transition:
      stroke-dashoffset 0.5s var(--ease-spring, cubic-bezier(0.34, 1.56, 0.64, 1)),
      stroke 0.3s ease;
  }

  .indexing-orbit-segment[data-phase-state='active'] {
    color: var(--primary-color);
    filter: drop-shadow(0 0 6px var(--primary-color));
  }

  .indexing-orbit-segment[data-phase-state='done'] {
    color: var(--primary-color);
    stroke-dashoffset: 0;
  }

  .indexing-orbit-segment[data-phase-state='error'] {
    color: var(--color-danger, oklch(55% 0.2 25deg));
  }

  .orbit-center-icon {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--primary-color);
    pointer-events: none;
    transition:
      transform 0.3s var(--ease-spring, cubic-bezier(0.34, 1.56, 0.64, 1)),
      opacity var(--transition-medium);
  }

  [data-ring-mode='hidden'] .orbit-center-icon {
    opacity: 0;
  }

  .orbit-center-icon :global(.feather) {
    width: var(--orbit-center-icon-size);
    height: var(--orbit-center-icon-size);
  }

  .orbit-dot {
    r: 5px;
    fill: var(--primary-color);
    filter: drop-shadow(0 0 4px var(--primary-color));
    animation: orbit-pulse 1.5s ease-in-out infinite;
  }

  @keyframes orbit-pulse {
    0%,
    100% {
      opacity: 1;
      r: 5px;
    }

    50% {
      opacity: 0.6;
      r: 7px;
    }
  }

  @keyframes orbit-segment {
    0% {
      transform: rotate(-28deg);
    }

    50% {
      transform: rotate(28deg);
    }

    100% {
      transform: rotate(-28deg);
    }
  }

  /* === Indexing Bottom Sheet === */
  .indexing-sheet-backdrop {
    position: fixed;
    inset: 0;
    z-index: 949;
    background: oklch(0% 0 0deg / 30%);
    opacity: 0;
    pointer-events: none;
    transition: opacity var(--transition-medium);
  }

  .indexing-sheet-backdrop.is-visible {
    opacity: 1;
    pointer-events: auto;
  }

  .indexing-bottom-sheet {
    position: fixed;
    bottom: 0;
    left: 50%;
    transform: translateX(-50%) translateY(100%);
    width: min(480px, 100%);
    max-height: 70vh;
    overflow-y: auto;
    z-index: 950;
    background: var(--glass-bg);
    backdrop-filter: blur(16px) saturate(1.5);
    -webkit-backdrop-filter: blur(16px) saturate(1.5);
    border: 1px solid var(--glass-border);
    border-radius: var(--radius-lg, 12px) var(--radius-lg, 12px) 0 0;
    padding: 0 0 env(safe-area-inset-bottom, 16px);
    transition: transform 0.4s var(--ease-spring, cubic-bezier(0.34, 1.56, 0.64, 1));
    will-change: transform;
  }

  .indexing-bottom-sheet[aria-hidden='false'] {
    transform: translateX(-50%) translateY(0);
  }

  .indexing-sheet-handle {
    width: 36px;
    height: 4px;
    background: var(--divider-color);
    border-radius: 2px;
    margin: 12px auto 0;
  }

  .indexing-sheet-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 16px 20px 8px;
    border-bottom: 1px solid var(--divider-color);
  }

  .indexing-sheet-title {
    font-size: 0.95rem;
    font-weight: 600;
    color: var(--text-primary);
  }

  .indexing-sheet-close {
    background: none;
    border: none;
    cursor: pointer;
    color: var(--text-secondary);
    padding: 4px;
    display: flex;
    align-items: center;
  }

  .indexing-sheet-summary {
    padding: 8px 20px;
    font-size: 0.85rem;
    color: var(--text-secondary);
  }

  .indexing-sheet-phases {
    padding: 8px 0;
  }

  .indexing-sheet-phase {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 20px;
    transition: background 0.2s ease;
  }

  .indexing-sheet-phase.is-active {
    background: oklch(from var(--primary-color) l c h / 8%);
    color: var(--primary-color);
  }

  .indexing-sheet-phase.is-done {
    opacity: 0.7;
  }

  .indexing-sheet-phase.is-error {
    color: var(--color-danger, oklch(55% 0.2 25deg));
  }

  :global(.phase-icon) {
    width: 16px;
    height: 16px;
    flex-shrink: 0;
    color: var(--text-secondary);
  }

  .indexing-sheet-phase.is-active :global(.phase-icon) {
    color: var(--primary-color);
  }

  .indexing-sheet-phase.is-done :global(.phase-icon) {
    color: var(--primary-color);
  }

  :global(.phase-info) {
    flex: 1;
    min-width: 0;
  }

  :global(.phase-name) {
    display: block;
    font-size: 0.85rem;
    color: var(--text-primary);
    margin-bottom: 4px;
  }

  :global(.phase-progress-bar) {
    height: 3px;
    background: var(--divider-color);
    border-radius: 2px;
    overflow: hidden;
  }

  :global(.phase-progress-fill) {
    height: 100%;
    background: var(--primary-color);
    border-radius: inherit;
    width: 0%;
    transition: width 0.3s ease;
  }

  :global(.phase-count) {
    font-size: 0.8rem;
    color: var(--text-secondary);
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
  }

  .indexing-sheet-current-item {
    padding: 8px 20px 16px;
    font-size: 0.75rem;
    color: var(--text-tertiary, var(--text-secondary));
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-height: 1.5em;
  }

  .indexing-phase-errors {
    font-size: 0.75rem;
    color: var(--color-danger, oklch(55% 0.2 25deg));
    margin-left: auto;
    white-space: nowrap;
  }

  @media (prefers-reduced-motion: reduce) {
    [data-phase-ring],
    .indexing-orbit-segment,
    .orbit-center-icon {
      transition: none;
    }

    .orbit-dot {
      animation: none;
    }
  }

  @media (width <= 768px) {
    [data-phase-ring] {
      --indexing-ring-large-size: 200px;
      --indexing-ring-compact-size: 48px;
    }
    [data-ring-mode='compact'] {
      left: 50%;
      right: auto;
      transform: translateX(-50%);
    }
    [data-ring-mode='hidden'] {
      left: 50%;
      right: auto;
      transform: translateX(-50%) scale(0.8);
    }
  }

  @media (width <= 480px) {
    [data-phase-ring] {
      --indexing-ring-compact-size: 40px;
    }
  }

  /* Solid-surface fallback: scoped so it outranks the base rule when
     backdrop-filter is unsupported or reduced transparency is requested. */
  @supports not (backdrop-filter: blur(1px)) {
    [data-phase-ring] {
      background: var(--surface-color);
    }
  }
</style>

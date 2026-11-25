<script>
  import Icon from './Icon.svelte';

  const {
    label,
    armedLabel,
    keyboardArmedLabel = armedLabel,
    busyLabel,
    ariaLabel,
    pending = false,
    onconfirm,
  } = $props();

  let trackEl = $state(null);
  let thumbEl = $state(null);
  let value = $state(0);
  let dragging = $state(false);
  let triggered = $state(false);
  let modality = $state('pointer');
  let pointerId = null;
  let startX = 0;
  let startValue = 0;
  let travel = 1;

  const KEYBOARD_KEYS = new Set([
    'ArrowRight',
    'ArrowUp',
    'ArrowLeft',
    'ArrowDown',
    'Home',
    'End',
    'Enter',
    ' ',
  ]);

  const busy = $derived(pending || triggered);
  const armed = $derived(value >= 100);
  const phase = $derived.by(() => {
    if (busy) return 'busy';
    if (armed) return 'armed';
    if (value === 0) return 'rest';
    return 'partial';
  });
  const armedText = $derived(modality === 'keyboard' ? keyboardArmedLabel : armedLabel);
  const valueText = $derived.by(() => {
    if (phase === 'busy') return busyLabel;
    if (phase === 'armed') return armedText;
    if (phase === 'rest') return label;
    return `${Math.round(value)} %`;
  });

  /** Reset the travel after a failed request; the dialog reopens at rest. */
  export function reset() {
    value = 0;
    triggered = false;
    dragging = false;
    modality = 'pointer';
    pointerId = null;
  }

  export function focus() {
    trackEl?.focus();
  }

  function onPointerDown(event) {
    if (busy || dragging || event.button !== 0) return;
    if (!thumbEl?.contains(event.target)) return;

    event.preventDefault();
    pointerId = event.pointerId;
    dragging = true;
    modality = 'pointer';
    startX = event.clientX;
    startValue = value;
    const inset = parseFloat(getComputedStyle(trackEl).getPropertyValue('--thumb-inset')) || 0;
    travel = Math.max(1, trackEl.clientWidth - thumbEl.offsetWidth - inset * 2);
    trackEl.setPointerCapture(event.pointerId);
    trackEl.focus();
  }

  function onPointerMove(event) {
    if (!dragging || event.pointerId !== pointerId) return;
    const next = startValue + ((event.clientX - startX) / travel) * 100;
    value = Math.min(100, Math.max(0, next));
  }

  function endDrag() {
    if (!dragging) return;
    dragging = false;
    if (pointerId !== null && trackEl?.hasPointerCapture(pointerId)) {
      trackEl.releasePointerCapture(pointerId);
    }
    pointerId = null;
  }

  function onPointerUp(event) {
    if (!dragging || event.pointerId !== pointerId) return;
    const completed = value >= 100;
    endDrag();
    if (completed) complete();
    else value = 0;
  }

  function onPointerCancel(event) {
    if (!dragging || event.pointerId !== pointerId) return;
    endDrag();
    value = 0;
  }

  function complete() {
    if (busy) return;
    value = 100;
    triggered = true;
    trackEl?.focus();
    onconfirm?.();
  }

  function onKeydown(event) {
    if (busy) return;
    // The armed copy follows the input that armed the slider: a keyboard arm
    // has no pointer to release, so it names Enter instead.
    if (KEYBOARD_KEYS.has(event.key)) modality = 'keyboard';

    switch (event.key) {
      case 'ArrowRight':
      case 'ArrowUp':
        event.preventDefault();
        event.stopPropagation();
        value = Math.min(100, value + 10);
        break;
      case 'ArrowLeft':
      case 'ArrowDown':
        event.preventDefault();
        event.stopPropagation();
        value = Math.max(0, value - 10);
        break;
      case 'Home':
        event.preventDefault();
        event.stopPropagation();
        value = 0;
        break;
      case 'End':
        event.preventDefault();
        event.stopPropagation();
        value = 100;
        break;
      case 'Enter':
      case ' ':
        event.preventDefault();
        event.stopPropagation();
        if (value >= 100) complete();
        break;
      default:
        break;
    }
  }
</script>

<div
  class="slide-confirm"
  class:armed
  class:busy
  class:dragging
  bind:this={trackEl}
  role="slider"
  tabindex="0"
  aria-label={ariaLabel}
  aria-orientation="horizontal"
  aria-valuemin="0"
  aria-valuemax="100"
  aria-valuenow={Math.round(value)}
  aria-valuetext={valueText}
  aria-disabled={busy}
  data-testid="confirm-dialog-slider"
  style="--progress: {value / 100}"
  onpointerdown={onPointerDown}
  onpointermove={onPointerMove}
  onpointerup={onPointerUp}
  onpointercancel={onPointerCancel}
  onlostpointercapture={onPointerCancel}
  onkeydown={onKeydown}
>
  <span class="slide-fill" aria-hidden="true"></span>
  <span class="slide-hint rest" class:hidden={phase !== 'rest'} aria-hidden="true">{label}</span>
  <span
    class="slide-hint state"
    class:visible={phase === 'armed' || phase === 'busy'}
    aria-hidden="true"
  >
    {phase === 'busy' ? busyLabel : armedText}
  </span>
  <span class="slide-thumb" bind:this={thumbEl} data-testid="confirm-dialog-slider-thumb">
    <Icon name="trash-2" width={18} height={18} />
  </span>
</div>

<style>
  .slide-confirm {
    --thumb-inset: 2px;
    --thumb-size: var(--button-size);

    position: relative;
    width: 100%;
    height: var(--button-size-lg);
    border-radius: var(--radius-full);
    background: var(--surface-elevated);
    box-shadow: inset 0 0 0 1px var(--divider-color);
    overflow: hidden;
    touch-action: none;
    user-select: none;
    font-family: var(--font-body);
    color: var(--text-secondary);
    transition: background-color var(--transition-fast);
  }

  .slide-confirm:focus-visible {
    outline: 2px solid var(--primary-color);
    outline-offset: 2px;
  }

  /* The trail is as tall as the thumb so its right cap matches the circle
     exactly; a full-height fill would peek out above and below it. The square
     left edge is clipped by the track's own pill. At rest the fill is fully
     transparent — a visible 0-progress slab reads as a square frame around the
     round thumb. */
  .slide-fill {
    position: absolute;
    top: var(--thumb-inset);
    bottom: var(--thumb-inset);
    left: 0;
    width: calc(
      var(--thumb-inset) + var(--thumb-size) + var(--progress) *
        (100% - var(--thumb-size) - 2 * var(--thumb-inset))
    );
    border-radius: 0 var(--radius-full) var(--radius-full) 0;
    background: var(--color-danger);
    opacity: var(--progress);
    pointer-events: none;
    transition:
      width var(--transition-fast),
      opacity var(--transition-fast);
  }

  .slide-confirm.dragging .slide-fill {
    transition: none;
  }

  /* At the far end the fill's opacity has already reached 1; flooding the
     track closes the last inset pixels and makes the whole pill the release
     signal. Busy is included because an external `pending` can arrive before
     the thumb reaches the end, and the busy label is white. */
  .slide-confirm.armed,
  .slide-confirm.busy {
    background: var(--color-danger);
  }

  .slide-hint {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    font-size: var(--font-sm);
    font-weight: var(--font-medium);
    white-space: nowrap;
    pointer-events: none;
    transition: opacity var(--transition-fast);
  }

  .slide-hint.rest.hidden,
  .slide-hint.state {
    opacity: 0;
  }

  .slide-hint.state.visible {
    opacity: 1;
  }

  .slide-confirm.armed .slide-hint.state,
  .slide-confirm.busy .slide-hint.state {
    color: white;
  }

  .slide-thumb {
    position: absolute;
    top: 50%;
    left: calc(
      var(--thumb-inset) + var(--progress) * (100% - var(--thumb-size) - 2 * var(--thumb-inset))
    );
    display: grid;
    place-items: center;
    width: var(--thumb-size);
    height: var(--thumb-size);
    border-radius: var(--radius-full);
    background: var(--color-danger);
    color: white;
    box-shadow: var(--shadow-medium);
    cursor: grab;
    transform: translateY(-50%);
    transition: left var(--transition-fast);
  }

  .slide-confirm.dragging .slide-thumb {
    cursor: grabbing;
    transition: none;
  }

  .slide-confirm.armed .slide-thumb,
  .slide-confirm.busy .slide-thumb {
    box-shadow:
      0 0 0 2px white,
      var(--shadow-medium);
  }

  .slide-confirm[aria-disabled='true'] .slide-thumb {
    cursor: wait;
    transition: none;
  }
</style>

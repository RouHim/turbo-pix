import { enterSelectionMode, selectionState, toggleSelected } from './state.svelte.js';
import { withRange } from './selectionRange.js';

/**
 * Shared state machine for the long-press range gesture on card surfaces.
 * Each surface owns one instance; the keys come from `selectionState.orderedKeys`,
 * which every card view already publishes for shift-click and select-all.
 */
export function createCardRangeSelection() {
  let anchor = null;

  return {
    begin(key) {
      if (selectionState.active) return false;
      anchor = key;
      enterSelectionMode();
      toggleSelected(key);
      return true;
    },

    move(key) {
      if (!anchor) return;
      const ordered = selectionState.orderedKeys;
      if (!ordered.includes(key)) return;

      const next = withRange(ordered, anchor, key);
      if (!next) return;
      selectionState.selected = next;
      selectionState.anchorKey = key;
    },

    end() {
      anchor = null;
    },
  };
}

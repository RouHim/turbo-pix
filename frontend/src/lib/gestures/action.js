import { GestureManager } from './GestureManager.js';

export function gestures(node, handlers) {
  const manager = new GestureManager(node, {
    enablePinch: true,
    enableSwipe: true,
    enableDoubleTap: true,
    enablePan: true,
  });

  // Store manager on node for external access (e.g. SwipeableViewer)
  node.__gestureManager = manager;

  function registerHandlers(h) {
    for (const [event, cb] of Object.entries(h || {})) {
      manager.on(event, cb);
    }
  }

  registerHandlers(handlers);
  manager.init();

  return {
    update(newHandlers) {
      // Drop callbacks for events the new handler set no longer provides;
      // on() overwrites per-event slots, so same-name handlers replace cleanly.
      for (const event of Object.keys(handlers || {})) {
        if (!newHandlers || !(event in newHandlers)) manager.off(event);
      }
      registerHandlers(newHandlers);
    },
    destroy() {
      node.__gestureManager = null;
      manager.destroy();
    },
  };
}

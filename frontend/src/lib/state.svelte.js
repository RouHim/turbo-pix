export const appState = $state({
  sidebarOpen: false,
  mobileSearchOpen: false,
});

export const indexingState = $state({
  isIndexing: false,
  currentPhase: null,
  phases: [],
  photosIndexed: 0,
  currentItem: '',
});

export const photoGridState = $state({
  photos: [],
  currentPage: 1,
  loading: false,
  hasMore: true,
  currentQuery: '',
  semanticSearchMode: false,
});

export const themeState = $state({ theme: 'light' });

export const toasts = $state([]);

let toastId = 0;
const TOAST_LEAVE_MS = 300;

export function addToast(title, message = '', type = 'info', duration = 4000) {
  const id = ++toastId;
  toasts.push({ id, title, message, type, leaving: false });
  if (duration > 0) {
    setTimeout(() => {
      removeToast(id);
    }, duration);
  }
}

export function removeToast(id) {
  const idx = toasts.findIndex((t) => t.id === id);
  if (idx === -1 || toasts[idx].leaving) return;
  // Play the leave animation, then actually remove.
  toasts[idx].leaving = true;
  setTimeout(() => {
    const i = toasts.findIndex((t) => t.id === id);
    if (i !== -1) toasts.splice(i, 1);
  }, TOAST_LEAVE_MS);
}

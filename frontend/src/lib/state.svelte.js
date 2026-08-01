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
  sheetOpen: false,
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
export function addToast(title, message = '', type = 'info', duration = 4000) {
  const id = ++toastId;
  toasts.push({ id, title, message, type });
  if (duration > 0) {
    setTimeout(() => {
      const idx = toasts.findIndex((t) => t.id === id);
      if (idx !== -1) toasts.splice(idx, 1);
    }, duration);
  }
}

export function removeToast(id) {
  const idx = toasts.findIndex((t) => t.id === id);
  if (idx !== -1) toasts.splice(idx, 1);
}

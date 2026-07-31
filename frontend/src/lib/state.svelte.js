export const appState = $state({
  currentView: 'all',
  searchQuery: '',
  sortOrder: 'date_desc',
  selectedYear: null,
  selectedMonth: null,
  isLoading: false,
  isMobile: false,
  sidebarOpen: false,
  mobileSearchOpen: false,
  totalPhotos: 0,
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

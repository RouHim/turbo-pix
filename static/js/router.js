class Router {
  constructor() {
    this.validViews = ['all', 'favorites', 'videos', 'collages', 'housekeeping'];
    this.validSorts = ['date_desc', 'date_asc', 'name_asc', 'name_desc', 'size_desc', 'size_asc'];
    this.defaultState = {
      view: 'all',
      photo: null,
      query: null,
      sort: 'date_desc',
      year: null,
      month: null,
    };
    this.listeners = [];
    this.isInitialized = false;
    this.currentState = this.getState();
    this.handlePopState = this.handlePopState.bind(this);
  }

  getState() {
    return this.parseUrl(new URL(window.location));
  }

  pushState(changes = {}) {
    return this.updateState('pushState', changes);
  }

  replaceState(changes = {}) {
    return this.updateState('replaceState', changes);
  }

  buildUrl(state = {}) {
    const normalizedState = this.normalizeState({ ...this.defaultState, ...state });
    const url = new URL(window.location.origin);

    url.pathname = normalizedState.view === 'all' ? '/' : `/${normalizedState.view}`;

    if (normalizedState.query) {
      url.searchParams.set('q', normalizedState.query);
    }

    if (normalizedState.sort !== this.defaultState.sort) {
      url.searchParams.set('sort', normalizedState.sort);
    }

    if (normalizedState.year !== null) {
      url.searchParams.set('year', String(normalizedState.year));

      if (normalizedState.month !== null) {
        url.searchParams.set('month', String(normalizedState.month));
      }
    }

    if (normalizedState.photo) {
      url.searchParams.set('photo', normalizedState.photo);
    }

    return `${url.pathname}${url.search}`;
  }

  onStateChange(callback) {
    this.listeners.push(callback);

    return () => {
      this.listeners = this.listeners.filter((listener) => listener !== callback);
    };
  }

  init() {
    if (!this.isInitialized) {
      window.addEventListener('popstate', this.handlePopState);
      this.isInitialized = true;
    }

    this.currentState = this.getState();
    return this.currentState;
  }

  handlePopState() {
    const previousState = this.currentState;
    const currentState = this.getState();

    this.currentState = currentState;
    this.listeners.forEach((listener) => {
      listener({ previous: previousState, current: currentState });
    });
  }

  updateState(historyMethod, changes) {
    const nextState = this.normalizeState({ ...this.getState(), ...changes });
    const url = this.buildUrl(nextState);

    window.history[historyMethod](nextState, '', url);
    this.currentState = nextState;

    return nextState;
  }

  parseUrl(url) {
    const pathView = url.pathname.replace(/^\//, '').replace(/\/$/, '');

    return this.normalizeState({
      view: pathView || 'all',
      photo: this.normalizeString(url.searchParams.get('photo')),
      query: this.normalizeString(url.searchParams.get('q')),
      sort: url.searchParams.get('sort'),
      year: this.parsePositiveInteger(url.searchParams.get('year')),
      month: this.parsePositiveInteger(url.searchParams.get('month')),
    });
  }

  normalizeState(state) {
    const view = this.validViews.includes(state.view) ? state.view : this.defaultState.view;
    const sort = this.validSorts.includes(state.sort) ? state.sort : this.defaultState.sort;

    return {
      view,
      photo: this.normalizeString(state.photo),
      query: this.normalizeString(state.query),
      sort,
      year: this.parsePositiveInteger(state.year),
      month: this.parsePositiveInteger(state.month),
    };
  }

  parsePositiveInteger(value) {
    if (value === null || value === undefined || value === '') {
      return null;
    }

    const parsedValue = Number.parseInt(value, 10);

    if (!Number.isInteger(parsedValue) || parsedValue <= 0) {
      return null;
    }

    return parsedValue;
  }

  normalizeString(value) {
    if (typeof value !== 'string') {
      return null;
    }

    const normalizedValue = value.trim();
    return normalizedValue ? normalizedValue : null;
  }
}

window.router = new Router();

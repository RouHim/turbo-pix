import { SvelteURL } from 'svelte/reactivity';

const validViews = ['all', 'favorites', 'videos', 'collages', 'housekeeping'];
const validSorts = ['date_desc', 'date_asc', 'name_asc', 'name_desc', 'size_desc', 'size_asc'];

const defaultState = {
  view: 'all',
  photo: null,
  query: null,
  sort: 'date_desc',
  year: null,
  month: null,
};

export const route = $state({ ...defaultState });

let updatingFromPopstate = false;
let isInitialized = false;

export function parseUrl(url) {
  const pathView = url.pathname.replace(/^\//, '').replace(/\/$/, '');

  return normalizeState({
    view: pathView || 'all',
    photo: normalizeString(url.searchParams.get('photo')),
    query: normalizeString(url.searchParams.get('q')),
    sort: url.searchParams.get('sort'),
    year: parsePositiveInteger(url.searchParams.get('year')),
    month: parsePositiveInteger(url.searchParams.get('month')),
  });
}

export function normalizeState(state) {
  const view = validViews.includes(state.view) ? state.view : defaultState.view;
  const sort = validSorts.includes(state.sort) ? state.sort : defaultState.sort;
  const year = parsePositiveInteger(state.year);
  const rawMonth = parsePositiveInteger(state.month);
  const month =
    year === null ? null : rawMonth !== null && rawMonth >= 1 && rawMonth <= 12 ? rawMonth : null;

  return {
    view,
    photo: normalizeString(state.photo),
    query: normalizeString(state.query),
    sort,
    year,
    month,
  };
}

export function parsePositiveInteger(value) {
  if (value === null || value === undefined || value === '') {
    return null;
  }

  const trimmed = String(value).trim();
  const parsedValue = Number.parseInt(trimmed, 10);

  if (!Number.isInteger(parsedValue) || parsedValue <= 0 || String(parsedValue) !== trimmed) {
    return null;
  }

  return parsedValue;
}

export function normalizeString(value) {
  if (typeof value !== 'string') {
    return null;
  }

  const normalizedValue = value.trim();
  return normalizedValue ? normalizedValue : null;
}

export function buildUrl(state = {}) {
  const normalizedState = normalizeState({ ...defaultState, ...state });
  const url = new SvelteURL(window.location.origin);

  url.pathname = normalizedState.view === 'all' ? '/' : `/${normalizedState.view}`;

  if (normalizedState.query) {
    url.searchParams.set('q', normalizedState.query);
  }

  if (normalizedState.sort !== defaultState.sort) {
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

function getCurrentState() {
  return parseUrl(new URL(window.location));
}

export function pushState(changes = {}) {
  const nextState = normalizeState({ ...getCurrentState(), ...changes });
  const url = buildUrl(nextState);

  if (!updatingFromPopstate) {
    window.history.pushState(nextState, '', url);
  }

  Object.assign(route, nextState);
}

export function replaceState(changes = {}) {
  const nextState = normalizeState({ ...getCurrentState(), ...changes });
  const url = buildUrl(nextState);

  if (!updatingFromPopstate) {
    window.history.replaceState(nextState, '', url);
  }

  Object.assign(route, nextState);
}

function handlePopState() {
  updatingFromPopstate = true;
  Object.assign(route, getCurrentState());
  // Svelte 5 effects flush in a microtask during the route mutation, so the
  // flag must survive that flush; clear it on the next microtask instead.
  queueMicrotask(() => {
    updatingFromPopstate = false;
  });
}

export function init() {
  if (!isInitialized) {
    window.addEventListener('popstate', handlePopState);
    isInitialized = true;
  }

  Object.assign(route, getCurrentState());
  return route;
}

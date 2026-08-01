import js from '@eslint/js';
import svelte from 'eslint-plugin-svelte';
import svelteParser from 'svelte-eslint-parser';

export default [
  js.configs.recommended,
  {
    linterOptions: {
      reportUnusedDisableDirectives: 'error',
    },
  },
  // Svelte 5 frontend
  ...svelte.configs['flat/recommended'],
  {
    files: ['frontend/src/**/*.svelte'],
    languageOptions: { parser: svelteParser },
  },
  {
    files: ['frontend/src/**/*.js', 'frontend/src/**/*.svelte'],
    languageOptions: {
      ecmaVersion: 2024, sourceType: 'module',
      globals: {
        window: 'readonly', document: 'readonly', console: 'readonly',
        fetch: 'readonly', URLSearchParams: 'readonly', URL: 'readonly',
        setTimeout: 'readonly', clearTimeout: 'readonly',
        setInterval: 'readonly', clearInterval: 'readonly',
        localStorage: 'readonly', navigator: 'readonly',
        queueMicrotask: 'readonly',
        CustomEvent: 'readonly', IntersectionObserver: 'readonly',
        Image: 'readonly', Blob: 'readonly', performance: 'readonly',
        AbortController: 'readonly', requestAnimationFrame: 'readonly',
        cancelAnimationFrame: 'readonly', HTMLCanvasElement: 'readonly',
        HTMLElement: 'readonly', HTMLVideoElement: 'readonly',
        HTMLImageElement: 'readonly', HTMLInputElement: 'readonly',
        HTMLDivElement: 'readonly', HTMLButtonElement: 'readonly',
        HTMLAnchorElement: 'readonly', FileReader: 'readonly',
        FormData: 'readonly', DOMParser: 'readonly', ResizeObserver: 'readonly',
        MutationObserver: 'readonly', MediaMetadata: 'readonly',
        SvelteURL: 'readonly',
        confirm: 'readonly',
      },
    },
    rules: {
      'no-console': 'off',
      'no-empty': 'warn',
      'no-undef': 'warn',
      'no-unused-vars': ['warn', { argsIgnorePattern: '^_' }],
      'prefer-const': 'warn',
      'no-var': 'error',
      'no-case-declarations': 'off',
      'no-redeclare': ['error', { builtinGlobals: false }],
      'svelte/no-at-html-tags': 'warn',
      'svelte/prefer-writable-derived': 'warn',
    },
  },
];

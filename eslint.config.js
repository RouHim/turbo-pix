import js from '@eslint/js';

export default [
  js.configs.recommended,
  {
    ignores: ['static/js/**/*.min.js'],
  },
  {
    files: ['static/js/**/*.js'],
    languageOptions: {
      ecmaVersion: 2022,
      sourceType: 'script',
      globals: {
        window: 'readonly',
        document: 'readonly',
        console: 'readonly',
        fetch: 'readonly',
        URLSearchParams: 'readonly',
        URL: 'readonly',
        setTimeout: 'readonly',
        clearTimeout: 'readonly',
        setInterval: 'readonly',
        localStorage: 'readonly',
        navigator: 'readonly',
        CustomEvent: 'readonly',
        IntersectionObserver: 'readonly',
        Image: 'readonly',
        Blob: 'readonly',
        performance: 'readonly',
        module: 'readonly',
        utils: 'readonly',
        api: 'readonly'
      }
    },
    rules: {
      'no-unused-vars': 'warn',
      'no-console': 'off',
      'prefer-const': 'error',
      'no-var': 'error',
      'no-case-declarations': 'off'
    }
  },
  {
    files: ['static/i18n/**/*.js', 'static/js/i18n.js', 'tests/**/*.js'],
    languageOptions: {
      ecmaVersion: 2022,
      sourceType: 'module',
      globals: {
        window: 'readonly',
        document: 'readonly',
        console: 'readonly',
        localStorage: 'readonly',
        navigator: 'readonly',
        module: 'readonly',
        global: 'writable',
        describe: 'readonly',
        test: 'readonly',
        expect: 'readonly',
        beforeEach: 'readonly',
        afterEach: 'readonly',
        jest: 'readonly'
      }
    },
    rules: {
      'no-unused-vars': 'off'
    }
  }
];
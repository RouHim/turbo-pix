import js from '@eslint/js';

export default [
  js.configs.recommended,
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
  }
];
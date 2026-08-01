import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

export default defineConfig({
  root: 'frontend',
  base: '/',
  plugins: [svelte()],
  build: {
    outDir: '../dist',
    emptyOutDir: true,
    sourcemap: false,
    // Lightning CSS minification collapses backdrop-filter + -webkit-backdrop-filter
    // pairs to the -webkit- form, which modern Chromium ignores (glassmorphism breaks).
    cssMinify: false,
    rollupOptions: {
      output: {
        entryFileNames: 'assets/[name].js',
        chunkFileNames: 'assets/[name].js',
        assetFileNames: 'assets/[name].[ext]',
      },
    },
  },
  server: {
    proxy: {
      '/api': 'http://localhost:18473',
      '/health': 'http://localhost:18473',
    },
  },
});

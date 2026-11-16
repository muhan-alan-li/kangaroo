// The two fixed entry names are what index.html and health.html reference, so the Python
// server and the tailscale serve config stay untouched. Shared code (Preact's runtime) goes
// into a content-hashed chunk: a rebuilt runtime can never be served from a browser's cache
// under an old name. `python3 -m server bump --bump` versions the entry URLs in the HTML.
import {defineConfig} from 'vite';

export default defineConfig({
  base: '/',
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    rollupOptions: {
      input: {
        postings: 'src/postings.tsx',
        health: 'src/health.tsx',
      },
      output: {
        entryFileNames: '[name].js',
        chunkFileNames: '[name]-[hash].js',
        assetFileNames: '[name].[ext]',
      },
    },
  },
});

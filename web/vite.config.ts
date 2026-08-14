import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'
import { TanStackRouterVite } from '@tanstack/router-plugin/vite'

export default defineConfig({
  plugins: [
    TanStackRouterVite({
      routesDirectory: './src/routes',
      generatedRouteTree: './src/routeTree.gen.ts',
    }),
    react(),
    tailwindcss(),
  ],
  build: {
    rollupOptions: {
      output: {
        // Keep the framework in its own long-lived chunk so an app-code change
        // doesn't invalidate it in the browser cache. The heavy preview
        // dependencies (video.js, CodeMirror, the markdown renderer) are split
        // out by the dynamic imports in `src/components/lazy.ts` instead of
        // being named here, so they load only when a preview opens.
        codeSplitting: {
          groups: [
            {
              name: "react",
              test: /node_modules[\\/](react|react-dom|scheduler)[\\/]/,
            },
            {
              name: "router",
              test: /node_modules[\\/]@tanstack[\\/]/,
            },
          ],
        },
      },
    },
  },
  server: {
    proxy: {
      '/api': {
        target: 'http://localhost:8080',
        changeOrigin: true,
      },
      '/auth': {
        target: 'http://localhost:8080',
        changeOrigin: true,
      },
      // Public share routes: /s/TOKEN/path — use rewrite to avoid matching /src/
      '^/s/[^/]+': {
        target: 'http://localhost:8080',
        changeOrigin: true,
      },
    },
  },
})

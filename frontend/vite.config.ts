import { defineConfig, type Plugin } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'
import { createHash } from 'node:crypto'
import { readFileSync } from 'node:fs'

/* Emits sw.js with this build's exact file list baked in. A changed list is a
   changed worker, which is how an installed home-screen app picks up a deploy
   and drops the previous build's cache. */
function serviceWorker(): Plugin {
  const statics = ['/', '/favicon.svg', '/manifest.webmanifest', '/manifest-dark.webmanifest', '/apple-touch-icon.png', '/apple-touch-icon-dark.png', '/icon-192.png', '/icon-192-dark.png', '/icon-512.png', '/icon-512-dark.png']
  return {
    name: 'own-audio-service-worker',
    apply: 'build',
    generateBundle(_options, bundle) {
      const assets = Object.keys(bundle).filter((f) => f.startsWith('assets/')).sort().map((f) => `/${f}`)
      const precache = [...statics, ...assets]
      const template = readFileSync(new URL('./sw/sw.js', import.meta.url), 'utf8')
      // The worker's own code counts too: a fix to it must replace caches it filled.
      const version = createHash('sha256').update(precache.join('\n')).update(template).digest('hex').slice(0, 12)
      const source = template
        .replaceAll('__VERSION__', version)
        .replaceAll('__PRECACHE__', JSON.stringify(precache))
      this.emitFile({ type: 'asset', fileName: 'sw.js', source })
    },
  }
}

// https://vite.dev/config/
export default defineConfig({
  plugins: [react(), tailwindcss(), serviceWorker()],
  server: {
    port: 5174,
    proxy: {
      // Proxy API calls and health endpoint to the Rust backend in dev.
      // 8080 matches BACKEND_PORT in docker-compose and SERVER__PORT in
      // backend/.env.example — override with VITE_DEV_API for a backend on
      // another port.
      '/api': process.env.VITE_DEV_API ?? 'http://127.0.0.1:8080',
      '/health': process.env.VITE_DEV_API ?? 'http://127.0.0.1:8080',
    },
  },
  build: {
    outDir: '../backend/ui/dist',
    emptyOutDir: true,
  },
})


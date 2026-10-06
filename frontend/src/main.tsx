// SPDX-License-Identifier: AGPL-3.0-or-later
import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import './index.css'
import Localized from './Localized.tsx'
import { initTheme } from './lib/theme'
import { initHomeScreenIcon } from './lib/homeScreenIcon'
import { initDownloads } from './lib/offline/downloads'
import { initI18n } from './i18n'

initTheme()
initI18n()
initHomeScreenIcon()

const queryClient = new QueryClient({
  defaultOptions: { queries: { retry: 1, staleTime: 30_000 } },
})

void initDownloads()

// Production only: a worker caching the dev server's modules would serve stale
// code under hot reload.
if (import.meta.env.PROD && 'serviceWorker' in navigator) {
  window.addEventListener('load', () => {
    navigator.serviceWorker.register('/sw.js').catch(() => {
      // Without it the app still works online; it just won't open offline.
    })
  })
}

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <Localized />
    </QueryClientProvider>
  </StrictMode>,
)

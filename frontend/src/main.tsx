import { render } from 'solid-js/web'
import { QueryClient, QueryClientProvider } from '@tanstack/solid-query'
import { RouterProvider, createRouter } from '@tanstack/solid-router'
import { routeTree } from './routeTree.gen'
import './app.css'

// Global keyboard shortcuts (real implementation)
window.addEventListener('keydown', (e) => {
  if (e.key.toLowerCase() === 'n' && (e.ctrlKey || e.metaKey)) {
    e.preventDefault()
    // Trigger new draft via custom event (listened in MailShell or Composer)
    window.dispatchEvent(new CustomEvent('komorebi:new-draft'))
  }
  if (e.key === '/' && document.activeElement?.tagName !== 'INPUT' && document.activeElement?.tagName !== 'TEXTAREA') {
    e.preventDefault()
    // Focus search (can be enhanced)
    const search = document.querySelector('input[placeholder*="Search"]') as HTMLInputElement
    search?.focus()
  }
})

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 1000 * 60 * 5, // 5 minutes
      retry: 1,
    },
  },
})

const router = createRouter({
  routeTree,
  context: {
    queryClient,
  },
  defaultPreload: 'intent',
  defaultPreloadStaleTime: 0,
})

// Register the router instance for type safety
declare module '@tanstack/solid-router' {
  interface Register {
    router: typeof router
  }
}

render(
  () => (
    <QueryClientProvider client={queryClient}>
      <RouterProvider router={router} />
    </QueryClientProvider>
  ),
  document.getElementById('root')!
)

import { createRootRoute, Outlet } from '@tanstack/solid-router'

export const Route = createRootRoute({
  component: () => (
    <div class="mail-shell">
      <Outlet />
    </div>
  ),
})

import { createRoute } from '@tanstack/solid-router'
import { MailShell } from '../mail/MailShell'
import { Route as rootRoute } from './__root'

export const Route = createRoute({
  getParentRoute: () => rootRoute,
  path: '/',
  component: () => <MailShell />,
})

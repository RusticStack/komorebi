// Manual route tree for the initial real implementation.
// TanStack Router will be happy with this for the first working shell.

import { Route as rootRoute } from './routes/__root'
import { Route as indexRoute } from './routes/index'

export const routeTree = rootRoute.addChildren([indexRoute])

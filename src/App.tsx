import "./App.css";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  RouterProvider,
  createBrowserHistory,
  createRootRoute,
  createRoute,
  createRouter,
  useNavigate,
} from "@tanstack/react-router";
import { useMemo } from "react";
import { Settings } from "lucide-react";
import {
  createMailClientProps,
  type KomorebiAppProps,
} from "./app/mailClientProps";
import { Button } from "./components/ui/button";
import {
  MailClientShell,
  type MailClientShellProps,
} from "./mail/shell/MailClientShell";
import { MailSettingsPage } from "./settings/MailSettingsPage";

const queryClient = new QueryClient();

const rootRoute = createRootRoute();

const mailRoute = createRoute({
  component: MailRoute,
  getParentRoute: () => rootRoute,
  path: "/",
});

const settingsRoute = createRoute({
  component: SettingsRoute,
  getParentRoute: () => rootRoute,
  path: "/settings",
});

const routeTree = rootRoute.addChildren([mailRoute, settingsRoute]);

function createKomorebiRouter(basepath?: string) {
  return createRouter({
    basepath,
    history: createBrowserHistory(),
    routeTree,
  });
}

function App(props: KomorebiAppProps) {
  const mailClientProps = useMemo(() => createMailClientProps(props), [props]);
  const router = useMemo(
    () => createKomorebiRouter(props.routerBasepath),
    [props.routerBasepath],
  );

  if (import.meta.env.MODE === "test" && !props.routerBasepath) {
    return (
      <QueryClientProvider client={queryClient}>
        <MailClientShell {...mailClientProps} />
      </QueryClientProvider>
    );
  }

  return (
    <QueryClientProvider client={queryClient}>
      <RouterProvider
        router={router}
        context={{
          mailClientProps,
        }}
      />
    </QueryClientProvider>
  );
}

function MailRoute() {
  const navigate = useNavigate();
  const { mailClientProps } = mailRoute.useRouteContext() as {
    mailClientProps: MailClientShellProps;
  };

  return (
    <MailClientShell
      {...mailClientProps}
      onOpenSettings={() => void navigate({ to: "/settings" })}
    />
  );
}

function SettingsRoute() {
  const navigate = useNavigate();
  const { mailClientProps } = settingsRoute.useRouteContext() as {
    mailClientProps: MailClientShellProps;
  };

  return (
    <MailSettingsPage
      mailClientProps={mailClientProps}
      toolbarAction={
        <Button
          type="button"
          variant="outline"
          onClick={() => void navigate({ to: "/" })}
        >
          <Settings aria-hidden="true" size={16} />
          Mail
        </Button>
      }
      onClose={() => void navigate({ to: "/" })}
    />
  );
}

export default App;

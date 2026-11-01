// Learn's own interface (Ferrum docs/plan/apps-ui.md) at /lms/: what a learner does (My learning,
// the course player with its quiz player, assignments and certificates, the catalog and course
// pages), what an instructor does (the teaching dashboard, grading, the course builder), and the
// pages anyone with a link opens (the public catalog, course pages and the certificate check).
// Everything else in Learn stays in the records frame, with Learn's modules in the sidebar. Every
// screen loads when it is opened, so the first script holds only the frame (apps-ui.md §12.9).

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { type ComponentType, StrictMode, useEffect } from "react";
import { createRoot } from "react-dom/client";
import { createBrowserRouter, Link, Outlet, RouterProvider, useRouteError } from "react-router";
import { isPublicPage, routeBase, shell } from "@ferrum/client";
import { ConfirmHost, EmptyState, ErrorState, Toaster } from "@ferrum/kit";
import { RequireAuth, setFrameMode } from "@ferrum/records/frame";
import "./styles.css";

/** A page of the records frame, loaded when first opened. */
const page = (name: "FormView" | "ListView" | "ReportView" | "AppSettingsPage" | "ScreenPage") => () =>
  import("./records").then((m) => ({ Component: m[name] }));

/** A screen of Learn's own, loaded when first opened. */
const screen = <K extends string>(load: () => Promise<Record<K, ComponentType>>, name: K) => () => load().then((m) => ({ Component: m[name] }));

/** Learn's packages: the sidebar shows their modules. */
const LMS_PACKAGES = ["lms"];

const queryClient = new QueryClient({
  defaultOptions: { queries: { retry: false, refetchOnWindowFocus: false, staleTime: 30_000 } },
});

function RouteError() {
  return <ErrorState error={useRouteError()} onRetry={() => window.location.reload()} />;
}

function NotFound() {
  return (
    <EmptyState title="Page not found">
      <Link to="/">Go to My learning</Link>
    </EmptyState>
  );
}

/** Every page tells the shell it started. */
function Started() {
  useEffect(() => shell().ready(), []);
  return null;
}

/** The pages anyone with a link opens: no sign-in, no sidebar, the workspace named in the address. */
function PublicFrame() {
  return (
    <main id="main" className="lms-public">
      <Started />
      <Outlet />
    </main>
  );
}

const publicRoutes = [
  {
    path: "/",
    element: <PublicFrame />,
    errorElement: <RouteError />,
    hydrateFallbackElement: <span />,
    children: [
      { path: "c/:workspace", lazy: screen(() => import("./Public"), "PublicCatalog") },
      { path: "c/:workspace/certificate/:code", lazy: screen(() => import("./Public"), "PublicCertificate") },
      { path: "c/:workspace/:slug", lazy: screen(() => import("./Public"), "PublicCourse") },
      { path: "c/:workspace/:slug/lessons/:lesson", lazy: screen(() => import("./Public"), "PublicLesson") },
      { path: "*", element: <NotFound /> },
    ],
  },
];

const appRoutes = [
  {
    path: "/",
    element: (
      <>
        <Started />
        <RequireAuth />
      </>
    ),
    errorElement: <RouteError />,
    children: [
      { index: true, lazy: screen(() => import("./Home"), "Home") },
      { path: "catalog", lazy: screen(() => import("./Catalog"), "Catalog") },
      { path: "course/:ref", lazy: screen(() => import("./Course"), "CourseRoute") },
      { path: "learn/:slug", lazy: screen(() => import("./Player"), "PlayerRoute") },
      { path: "learn/:slug/:lesson", lazy: screen(() => import("./Player"), "PlayerRoute") },
      { path: "teaching", lazy: screen(() => import("./Teaching"), "Teaching") },
      { path: "teaching/:course", lazy: screen(() => import("./CourseDashboard"), "CourseDashboardRoute") },
      { path: "build/:course", lazy: screen(() => import("./Builder"), "BuilderRoute") },
      { path: "e/:entity", lazy: page("ListView") },
      { path: "e/:entity/:id", lazy: page("FormView") },
      { path: "r/:name", lazy: page("ReportView") },
      { path: "apps/:app/settings", lazy: page("AppSettingsPage") },
      { path: "x/:app/:page", lazy: page("ScreenPage") },
      { path: "*", element: <NotFound /> },
    ],
  },
];

// A public page is the same release opened below /lms/c/ with no session: it needs none of the frame.
const publicPage = isPublicPage();
if (!publicPage) setFrameMode("app", LMS_PACKAGES);

const router = createBrowserRouter(publicPage ? publicRoutes : appRoutes, { basename: routeBase("lms") });

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <RouterProvider router={router} />
      <Toaster />
      <ConfirmHost />
    </QueryClientProvider>
  </StrictMode>,
);

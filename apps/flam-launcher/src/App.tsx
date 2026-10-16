import { lazy, Suspense } from "react";
import { ToastProvider } from "./components";
import { AppFrame } from "./features/app/AppFrame";
import { CompatProvider } from "./features/compat/CompatProvider";
import { LibraryProvider } from "./features/library/LibraryProvider";
import { RecoveryProvider } from "./features/recovery/RecoveryProvider";
import { StoriesProvider } from "./features/stories/StoriesProvider";
import { DEMO_HASH, useRoute } from "./router";

export { DEMO_HASH };

// `import.meta.env.DEV` vaut false au build : Vite retire alors la page et son import.
const DemoPage = import.meta.env.DEV
  ? lazy(() => import("./features/demo/DemoPage").then((m) => ({ default: m.DemoPage })))
  : null;

export function App() {
  const route = useRoute();

  if (route === "demo") {
    if (!DemoPage) return <AppRoutes route="home" />;
    return (
      <Suspense fallback={null}>
        <DemoPage />
      </Suspense>
    );
  }
  return <AppRoutes route={route} />;
}

function AppRoutes({ route }: { route: "home" | "sources" | "settings" | "library" }) {
  return (
    <ToastProvider>
      <RecoveryProvider>
        <LibraryProvider>
          <CompatProvider>
            <StoriesProvider>
              <AppFrame route={route} />
            </StoriesProvider>
          </CompatProvider>
        </LibraryProvider>
      </RecoveryProvider>
    </ToastProvider>
  );
}

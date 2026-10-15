import { lazy, Suspense, useSyncExternalStore } from "react";
import { Welcome } from "./features/welcome/Welcome";
import { t } from "./i18n";

/** Adresse de la page de démonstration des composants (développement uniquement). */
export const DEMO_HASH = "#/demo";

// `import.meta.env.DEV` vaut false au build : Vite retire alors la page et son import.
const DemoPage = import.meta.env.DEV
  ? lazy(() => import("./features/demo/DemoPage").then((m) => ({ default: m.DemoPage })))
  : null;

function subscribeHash(callback: () => void) {
  window.addEventListener("hashchange", callback);
  return () => window.removeEventListener("hashchange", callback);
}

export function App() {
  const hash = useSyncExternalStore(subscribeHash, () => window.location.hash);

  if (DemoPage && hash === DEMO_HASH) {
    return (
      <Suspense fallback={null}>
        <DemoPage />
      </Suspense>
    );
  }

  return (
    <main className="app">
      <Welcome />
      {DemoPage && <a href={DEMO_HASH}>{t.demo.link}</a>}
    </main>
  );
}

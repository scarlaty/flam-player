import { useEffect, useRef } from "react";
import { getSettings } from "../../api/settings";
import { listSources } from "../../api/sources";
import { AppShell } from "../../components";
import { t } from "../../i18n";
import type { Route } from "../../router";
import { applyTheme } from "../../theme/theme";
import { CompatNotice } from "../compat/CompatNotice";
import { DeviceScreen } from "../device/DeviceScreen";
import { TransferPanel } from "../device/TransferPanel";
import { DeviceLibrarySend } from "../stories/DeviceLibrarySend";
import { LibrarySidebar } from "../library/LibrarySidebar";
import { PlayerScreen } from "../player/PlayerScreen";
import { RecoveryBanners } from "../recovery/RecoveryBanners";
import { useRecovery } from "../recovery/RecoveryContext";
import { StoriesLibrary } from "../stories/StoriesLibrary";
import { useStories } from "../stories/StoriesContext";
import { StoriesTaskPanel } from "../stories/StoriesTaskPanel";
import { SettingsScreen } from "../settings/SettingsScreen";
import { SourcesScreen } from "../sources/SourcesScreen";
import { Welcome } from "../welcome/Welcome";
import { AppNav } from "./AppNav";

const TITLES: Record<Exclude<Route, "demo">, string> = {
  home: t.welcome.title,
  sources: t.nav.sources,
  settings: t.nav.settings,
  library: t.nav.library,
  device: t.nav.device,
};

/**
 * Au démarrage : applique le thème enregistré et relève les restaurations de configuration
 * à signaler (les erreurs sont affichées par les écrans eux-mêmes).
 */
function useStartupConfig() {
  const { report } = useRecovery();
  useEffect(() => {
    getSettings()
      .then((view) => {
        applyTheme(view.settings.theme);
        report("settings.json", view.recovery);
      })
      .catch(() => undefined);
    listSources()
      .then((view) => report("sources.json", view.recovery))
      .catch(() => undefined);
  }, [report]);
}

/**
 * Mise en page commune des écrans : navigation, titre, bandeaux de restauration. Tant qu'un
 * player incrusté tourne, l'écran Lecture remplace toute l'application (pas de navigation
 * pendant la lecture : la fenêtre native du player resterait au-dessus des autres écrans).
 */
export function AppFrame({ route }: { route: Exclude<Route, "demo"> }) {
  useStartupConfig();
  const { session, forgetSession } = useStories();
  const embedded = session?.embedded === true;
  // Fin de l'écran Lecture : le focus va au contenu principal (sinon il serait perdu sur un
  // élément démonté).
  const wasEmbedded = useRef(false);
  useEffect(() => {
    if (wasEmbedded.current && !embedded) document.getElementById("fl-main")?.focus();
    wasEmbedded.current = embedded;
  }, [embedded]);
  if (session && embedded)
    return <PlayerScreen session={session} onGone={() => forgetSession(session.pid)} />;
  return (
    <AppShell
      navLabel={t.nav.label}
      title={TITLES[route]}
      aside={
        <>
          <StoriesTaskPanel />
          <TransferPanel />
        </>
      }
      sidebar={
        <>
          <AppNav current={route} />
          <LibrarySidebar />
        </>
      }
    >
      <RecoveryBanners />
      {route === "sources" && <SourcesScreen />}
      {route === "settings" && <SettingsScreen />}
      {route === "device" && <DeviceScreen libraryPanel={<DeviceLibrarySend />} />}
      {route === "library" && (
        <>
          <CompatNotice />
          <StoriesLibrary />
        </>
      )}
      {route === "home" && <Welcome />}
    </AppShell>
  );
}

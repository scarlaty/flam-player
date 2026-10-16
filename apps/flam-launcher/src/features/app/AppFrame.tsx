import { useEffect } from "react";
import { getSettings } from "../../api/settings";
import { listSources } from "../../api/sources";
import { AppShell } from "../../components";
import { t } from "../../i18n";
import type { Route } from "../../router";
import { applyTheme } from "../../theme/theme";
import { RecoveryBanners } from "../recovery/RecoveryBanners";
import { useRecovery } from "../recovery/RecoveryContext";
import { SettingsScreen } from "../settings/SettingsScreen";
import { SourcesScreen } from "../sources/SourcesScreen";
import { Welcome } from "../welcome/Welcome";
import { AppNav } from "./AppNav";

const TITLES: Record<Exclude<Route, "demo">, string> = {
  home: t.welcome.title,
  sources: t.nav.sources,
  settings: t.nav.settings,
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

/** Mise en page commune des écrans : navigation, titre, bandeaux de restauration. */
export function AppFrame({ route }: { route: Exclude<Route, "demo"> }) {
  useStartupConfig();
  return (
    <AppShell
      navLabel={t.nav.label}
      title={TITLES[route]}
      sidebar={<AppNav current={route} />}
    >
      <RecoveryBanners />
      {route === "sources" && <SourcesScreen />}
      {route === "settings" && <SettingsScreen />}
      {route === "home" && <Welcome />}
    </AppShell>
  );
}

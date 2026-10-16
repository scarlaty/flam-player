import { useEffect, useState } from "react";
import { getAppInfo, type AppInfo } from "../../api/appInfo";
import { t } from "../../i18n";
import { demoFr } from "../../i18n/demo.fr";
import { DEMO_HASH } from "../../router";

/** Contenu de l'accueil (le titre « Bienvenue » est celui de la mise en page). */
export function Welcome() {
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    let active = true;
    getAppInfo()
      .then((value) => {
        if (active) setInfo(value);
      })
      .catch(() => {
        if (active) setFailed(true);
      });
    return () => {
      active = false;
    };
  }, []);

  return (
    <section>
      {info && <p>{t.welcome.version(info.name, info.version)}</p>}
      {failed && <p role="alert">{t.welcome.error}</p>}
      {import.meta.env.DEV && <a href={DEMO_HASH}>{demoFr.link}</a>}
    </section>
  );
}

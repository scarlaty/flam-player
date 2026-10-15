import { useEffect, useState } from "react";
import { getAppInfo, type AppInfo } from "../../api/appInfo";

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
      <h1>Bienvenue</h1>
      {info && (
        <p>
          {info.name} — version {info.version}
        </p>
      )}
      {failed && (
        <p role="alert">Impossible de lire les informations de l'application.</p>
      )}
    </section>
  );
}

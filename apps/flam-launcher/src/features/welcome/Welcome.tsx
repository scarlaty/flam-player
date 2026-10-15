import { useEffect, useState } from "react";
import { getAppInfo, type AppInfo } from "../../api/appInfo";
import { t } from "../../i18n";

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
      <h1>{t.welcome.title}</h1>
      {info && <p>{t.welcome.version(info.name, info.version)}</p>}
      {failed && <p role="alert">{t.welcome.error}</p>}
    </section>
  );
}

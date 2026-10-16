import { useState } from "react";
import { Banner, Button } from "../../components";
import { t } from "../../i18n";
import { useRecovery } from "./RecoveryContext";

/** Bandeaux d'avertissement : un fichier de configuration corrompu a été restauré. */
export function RecoveryBanners() {
  const { recoveries, acknowledge } = useRecovery();
  const [failed, setFailed] = useState<string | null>(null);

  if (recoveries.length === 0) return null;

  async function ack(file: string) {
    try {
      await acknowledge(file);
      setFailed(null);
    } catch {
      setFailed(file);
    }
  }

  return (
    <div className="fl-recovery">
      {recoveries.map((recovery) => (
        <Banner
          key={recovery.file}
          tone="warning"
          title={t.recovery.title(recovery.file)}
          action={
            <Button size="sm" onClick={() => ack(recovery.file)}>
              {t.recovery.ack}
            </Button>
          }
        >
          <p>{t.recovery.body(recovery.backup)}</p>
          {failed === recovery.file && <p role="alert">{t.recovery.ackFailed}</p>}
        </Banner>
      ))}
    </div>
  );
}

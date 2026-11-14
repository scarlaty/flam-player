import { useEffect, useState } from "react";
import {
  cancelClose,
  getCloseRetained,
  onCloseCleared,
  onCloseRetained,
  type WriteOperation,
} from "../../api/device";
import { Banner, Button } from "../../components";
import { t } from "../../i18n";

/**
 * Bandeau non modal affiché quand le backend retient la fermeture de la fenêtre pendant une
 * écriture sur la Flam (#577). La fenêtre se ferme d'elle-même à la fin de l'opération en cours
 * (les envois en attente ne partent pas) ; une deuxième demande de fermeture ferme tout de suite ; « Ne pas fermer » lève la retenue (un clic
 * par erreur sur la croix ne ferme pas l'application vingt minutes plus tard). Le bandeau
 * disparaît quand le backend signale la fin de la retenue.
 */
export function CloseRetainedBanner() {
  const [operation, setOperation] = useState<WriteOperation | null>(null);

  useEffect(() => {
    let disposed = false;
    const offs: Array<() => void> = [];
    const keep = (off: () => void) => {
      if (disposed) off();
      else offs.push(off);
    };
    void onCloseRetained((event) => setOperation(event.operation))
      .then(keep)
      .catch(() => undefined);
    void onCloseCleared(() => setOperation(null))
      .then(keep)
      .catch(() => undefined);
    // Rattrapage : une retenue déjà en cours (événement émis avant le montage, écran de lecture).
    void getCloseRetained()
      .then((retained) => {
        if (!disposed && retained) setOperation((current) => current ?? retained);
      })
      .catch(() => undefined);
    return () => {
      disposed = true;
      offs.forEach((off) => off());
    };
  }, []);

  if (operation === null) return null;
  return (
    <div className="fl-recovery">
      <Banner
        tone="warning"
        role="status"
        title={t.closeRetained.title}
        action={
          <Button
            size="sm"
            onClick={() => {
              // La retenue est levée côté backend ; le bandeau disparaît même si l'appel échoue
              // (la fermeture se fait alors à la fin, comme demandé).
              void cancelClose().catch(() => undefined);
              setOperation(null);
            }}
          >
            {t.closeRetained.keep}
          </Button>
        }
      >
        <p>{t.closeRetained.body}</p>
        <p>{t.closeRetained.resume[operation]}</p>
      </Banner>
    </div>
  );
}

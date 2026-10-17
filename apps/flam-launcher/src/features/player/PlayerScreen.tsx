import { useEffect, useRef, useState, type RefObject } from "react";
import { toAppError } from "../../api/error";
import {
  focusEmbeddedPlayer,
  onPlayerFocusHost,
  setPlayerEmbedLayout,
  stopPlayer,
  type EmbedRect,
  type PlayerSession,
} from "../../api/player";
import { Button } from "../../components";
import { t } from "../../i18n";
import "./PlayerScreen.css";

/**
 * Délai au-delà duquel « Arrêter » est rendu de nouveau disponible si la fin du player n'est
 * pas arrivée (le backend force l'arrêt après 5 s) : l'écran a toujours une issue.
 */
export const STOP_TIMEOUT_MS = 10_000;

/**
 * Attente de `player://exited` quand `player_stop` répond qu'aucun player ne tourne : la fin
 * (rapatriement des sauvegardes) peut être en cours. Sans événement, l'écran se ferme.
 */
export const GONE_GRACE_MS = 3_000;

function sameRect(a: EmbedRect | null, b: EmbedRect): boolean {
  return (
    a !== null &&
    a.x === b.x &&
    a.y === b.y &&
    a.width === b.width &&
    a.height === b.height
  );
}

/**
 * Suit la position et la taille de `zone` et les transmet au backend, qui y place la fenêtre
 * du player incrusté (fenêtre native posée au-dessus de la page). Au démontage, la fenêtre est
 * cachée.
 */
function useEmbedZone(zone: RefObject<HTMLDivElement | null>) {
  useEffect(() => {
    const element = zone.current;
    if (!element) return;
    let sent: EmbedRect | null = null;
    let frame = 0;
    const send = () => {
      frame = 0;
      const box = element.getBoundingClientRect();
      const rect = { x: box.left, y: box.top, width: box.width, height: box.height };
      if (sameRect(sent, rect)) return;
      sent = rect;
      setPlayerEmbedLayout(rect).catch(() => undefined);
    };
    // Une seule mesure par image, même si plusieurs changements arrivent.
    const schedule = () => {
      if (frame === 0) frame = requestAnimationFrame(send);
    };
    send();
    const observer = new ResizeObserver(schedule);
    observer.observe(element);
    window.addEventListener("resize", schedule);
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", schedule);
      if (frame !== 0) cancelAnimationFrame(frame);
      setPlayerEmbedLayout(null).catch(() => undefined);
    };
  }, [zone]);
}

/**
 * Écran Lecture : affiché à la place de l'application tant qu'un player incrusté tourne. La
 * zone d'accueil reçoit la fenêtre du player ; « Arrêter » le termine proprement.
 *
 * Les erreurs sont affichées dans l'en-tête (`role="alert"`), jamais par une notification : la
 * fenêtre native du player, posée au-dessus de la page, pourrait la masquer. `onGone` est
 * appelé si le backend répond qu'aucun player ne tourne (fin de partie manquée).
 */
export function PlayerScreen({
  session,
  onGone,
}: {
  session: PlayerSession;
  onGone: () => void;
}) {
  const zone = useRef<HTMLDivElement>(null);
  const heading = useRef<HTMLHeadingElement>(null);
  const focusButton = useRef<HTMLButtonElement>(null);
  const [stopping, setStopping] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [gone, setGone] = useState(false);
  useEmbedZone(zone);

  // Aucun player côté backend : sans fin de partie dans le délai, l'écran se ferme.
  useEffect(() => {
    if (!gone) return;
    const timer = window.setTimeout(onGone, GONE_GRACE_MS);
    return () => window.clearTimeout(timer);
  }, [gone, onGone]);

  // Focus au montage : l'élément qui a lancé la lecture vient d'être démonté.
  useEffect(() => {
    heading.current?.focus();
  }, []);

  // F6 dans le player : le clavier revient au launcher, sur « Clavier au lecteur » (qui le
  // redonne au player). Le désabonnement tient compte d'un démontage avant l'abonnement.
  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | undefined;
    onPlayerFocusHost(() => focusButton.current?.focus())
      .then((off) => {
        if (active) unlisten = off;
        else off();
      })
      .catch(() => undefined);
    return () => {
      active = false;
      try {
        // Rejet possible si les ponts Tauri sont déjà retirés : rien à désabonner alors.
        (unlisten?.() as unknown as Promise<void> | undefined)?.catch?.(() => undefined);
      } catch {
        // Idem, en erreur synchrone.
      }
    };
  }, []);

  // Filet : sans fin de partie après le délai, « Arrêter » redevient disponible.
  useEffect(() => {
    if (!stopping) return;
    const timer = window.setTimeout(() => {
      setStopping(false);
      setError(t.playerScreen.stopTimeout);
    }, STOP_TIMEOUT_MS);
    return () => window.clearTimeout(timer);
  }, [stopping]);

  function stop() {
    setStopping(true);
    setError(null);
    stopPlayer()
      .then((running) => {
        // Aucun player côté backend : fin en cours ou manquée.
        if (!running) setGone(true);
      })
      .catch((caught: unknown) => {
        setStopping(false);
        setError(`${t.playerScreen.stopFailed} ${toAppError(caught).message}`);
      });
  }

  return (
    <div className="fl-player-screen">
      <header className="fl-player-screen__header">
        <h1 ref={heading} className="fl-player-screen__title" tabIndex={-1}>
          {session.title}
        </h1>
        <div className="fl-player-screen__actions">
          <Button
            ref={focusButton}
            variant="secondary"
            icon="play"
            onClick={() => {
              focusEmbeddedPlayer().catch(() => undefined);
            }}
          >
            {t.playerScreen.focus}
          </Button>
          <Button variant="danger" icon="close" onClick={stop} disabled={stopping}>
            {stopping ? t.playerScreen.stopping : t.playerScreen.stop}
          </Button>
        </div>
      </header>
      {error && (
        <p className="fl-player-screen__error" role="alert">
          {error}
        </p>
      )}
      <main className="fl-player-screen__main" aria-label={t.playerScreen.label}>
        <div
          ref={zone}
          className="fl-player-screen__zone"
          role="region"
          aria-label={t.playerScreen.zone(session.title)}
        />
        <p className="fl-player-screen__hint">{t.playerScreen.hint}</p>
      </main>
    </div>
  );
}

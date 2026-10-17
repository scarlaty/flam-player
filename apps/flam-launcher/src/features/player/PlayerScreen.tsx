import { useEffect, useRef, useState, type RefObject } from "react";
import { toAppError } from "../../api/error";
import {
  setPlayerEmbedLayout,
  stopPlayer,
  type EmbedRect,
  type PlayerSession,
} from "../../api/player";
import { Button, useToast } from "../../components";
import { t } from "../../i18n";
import "./PlayerScreen.css";

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
 */
export function PlayerScreen({ session }: { session: PlayerSession }) {
  const zone = useRef<HTMLDivElement>(null);
  const [stopping, setStopping] = useState(false);
  const toast = useToast();
  useEmbedZone(zone);

  function stop() {
    setStopping(true);
    stopPlayer().catch((caught: unknown) => {
      setStopping(false);
      toast.show({
        message: `${t.playerScreen.stopFailed} ${toAppError(caught).message}`,
        tone: "danger",
      });
    });
  }

  return (
    <div className="fl-player-screen">
      <header className="fl-player-screen__header">
        <h1 className="fl-player-screen__title">{session.title}</h1>
        <Button variant="danger" icon="close" onClick={stop} disabled={stopping}>
          {stopping ? t.playerScreen.stopping : t.playerScreen.stop}
        </Button>
      </header>
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

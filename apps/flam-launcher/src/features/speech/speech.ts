import { useCallback, useEffect, useRef, useState } from "react";
import { toAppError, type AppError } from "../../api/error";
import {
  listVoices,
  previewSpeech,
  stopSpeechPreview,
  type VoiceList,
} from "../../api/speech";
import { t } from "../../i18n";
import { ownText } from "../../i18n/lookup";

/** Voix françaises du poste, lues une fois au montage (`tts_voices`). */
export function useVoices(): { list: VoiceList | null; error: AppError | null } {
  const [list, setList] = useState<VoiceList | null>(null);
  const [error, setError] = useState<AppError | null>(null);
  useEffect(() => {
    let cancelled = false;
    listVoices()
      .then((voices) => {
        if (!cancelled) setList(voices);
      })
      .catch((caught: unknown) => {
        if (!cancelled) setError(toAppError(caught));
      });
    return () => {
      cancelled = true;
    };
  }, []);
  return { list, error };
}

/** Texte d'une erreur d'aperçu : table par code (`Object.hasOwn`), sinon le message du backend. */
export function previewErrorMessage(error: AppError): string {
  return ownText(t.speech.preview, error.kind) ?? error.message;
}

export interface SpeechPreview {
  /** Aperçu demandé ou en cours de lecture. */
  playing: boolean;
  error: AppError | null;
  play: (text: string, voice: string | null) => void;
  stop: () => void;
}

/**
 * Aperçu d'une voix (`tts_preview`, joué par le backend). `playing` reste vrai pendant la
 * synthèse puis pendant la durée du son ; un nouvel aperçu remplace le précédent. L'aperçu est
 * coupé quand le composant disparaît.
 */
export function useSpeechPreview(): SpeechPreview {
  const [playing, setPlaying] = useState(false);
  const [error, setError] = useState<AppError | null>(null);
  // Numéro du dernier aperçu demandé : une réponse plus ancienne est ignorée.
  const request = useRef(0);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  // Un aperçu a été demandé et ni arrêté ni terminé : le backend peut encore jouer.
  const active = useRef(false);

  const clearTimer = () => {
    if (timer.current !== null) clearTimeout(timer.current);
    timer.current = null;
  };

  const stop = useCallback(() => {
    request.current += 1;
    clearTimer();
    setPlaying(false);
    if (active.current) {
      active.current = false;
      stopSpeechPreview().catch(() => undefined);
    }
  }, []);

  const play = useCallback((text: string, voice: string | null) => {
    const mine = ++request.current;
    clearTimer();
    setError(null);
    setPlaying(true);
    active.current = true;
    const finish = () => {
      if (mine !== request.current) return;
      active.current = false;
      setPlaying(false);
    };
    previewSpeech(text, voice)
      .then((info) => {
        if (mine !== request.current) return;
        if (!info.played) finish();
        else timer.current = setTimeout(finish, info.duration_ms);
      })
      .catch((caught: unknown) => {
        if (mine !== request.current) return;
        finish();
        setError(toAppError(caught));
      });
  }, []);

  useEffect(
    () => () => {
      request.current += 1;
      clearTimer();
      if (active.current) {
        active.current = false;
        stopSpeechPreview().catch(() => undefined);
      }
    },
    [],
  );

  return { playing, error, play, stop };
}

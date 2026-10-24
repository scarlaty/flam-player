import { invokeCommand } from "./error";

/**
 * Synthèse vocale des titres des packs (#462, miroir de `src-tauri/src/podcast/speech.rs`).
 * Tout reste sur le poste : voix de Windows, aperçu joué par le backend (aucun son transmis à
 * l'interface).
 */

/** Voix française installée (`Voice`). */
export interface Voice {
  /** Identifiant stable, mémorisé dans le réglage `tts_voice`. */
  id: string;
  /** Nom affiché (« Microsoft Hortense »). */
  name: string;
  /** Étiquette de langue (`fr-FR`). */
  language: string;
}

/** Voix françaises du poste (`VoiceList`). */
export interface VoiceList {
  /** Triées par nom ; vide : aucune voix française, ou synthèse indisponible. */
  voices: Voice[];
  /** Voix proposée par défaut ; `null` si la liste est vide. */
  default_id: string | null;
}

/** Résultat d'un aperçu (`PreviewInfo`). */
export interface PreviewInfo {
  /** `false` : un aperçu plus récent (ou un arrêt) l'a rendu inutile, rien n'est joué. */
  played: boolean;
  /** Durée du son joué, en millisecondes (0 si rien n'est joué). */
  duration_ms: number;
}

/** `tts_voices` : voix françaises installées (liste vide si aucune). */
export function listVoices(): Promise<VoiceList> {
  return invokeCommand<VoiceList>("tts_voices");
}

/**
 * `tts_preview` : lit `text` avec `voice` (voix par défaut si absente) sur la sortie audio du
 * poste. Un seul aperçu à la fois : celui-ci coupe le précédent. Rend la main quand la lecture
 * commence.
 */
export function previewSpeech(text: string, voice?: string | null): Promise<PreviewInfo> {
  return invokeCommand<PreviewInfo>("tts_preview", { text, voice: voice || null });
}

/** `tts_preview_stop` : coupe l'aperçu en cours et annule celui qui se prépare. */
export function stopSpeechPreview(): Promise<void> {
  return invokeCommand<void>("tts_preview_stop");
}

/**
 * Voix à utiliser : `wanted` si elle est encore installée, sinon la voix par défaut (même repli
 * que le backend). `null` si aucune voix.
 */
export function effectiveVoice(list: VoiceList, wanted?: string | null): string | null {
  if (wanted && list.voices.some((v) => v.id === wanted)) return wanted;
  return list.default_id ?? list.voices[0]?.id ?? null;
}

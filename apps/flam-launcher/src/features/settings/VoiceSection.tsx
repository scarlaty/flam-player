import { effectiveVoice } from "../../api/speech";
import { Banner, Button, SelectField } from "../../components";
import { t } from "../../i18n";
import { previewErrorMessage, useSpeechPreview, useVoices } from "../speech/speech";

export interface VoiceSectionProps {
  /** Voix mémorisée (`tts_voice`) ; `null` : voix par défaut. */
  value: string | null;
  /** Enregistre la voix choisie (identifiant d'une voix de la liste). */
  onChange: (voice: string) => void;
  /** L'enregistrement de la voix a échoué. */
  failed: boolean;
}

/**
 * Paramètres : voix des titres lus des packs (#462, note § 12.4) parmi les voix françaises du
 * poste, et bouton d'essai. Pas de réglage de débit (v1).
 */
export function VoiceSection({ value, onChange, failed }: VoiceSectionProps) {
  const text = t.speech;
  const { list, error } = useVoices();
  const preview = useSpeechPreview();
  const voices = list?.voices ?? [];
  const used = list ? effectiveVoice(list, value) : null;
  const gone = value !== null && voices.length > 0 && used !== value;

  return (
    <section className="fl-settings__section" aria-labelledby="settings-voice">
      <h2 id="settings-voice">{text.settings.title}</h2>
      <p>{text.settings.note}</p>
      {failed && <Banner tone="danger">{text.settings.failed}</Banner>}
      {list === null && error === null ? (
        <p role="status">{text.loading}</p>
      ) : error ? (
        <p>{text.listFailed}</p>
      ) : voices.length === 0 ? (
        <p>{text.noVoice}</p>
      ) : (
        <>
          {gone && <p>{text.settings.gone}</p>}
          <SelectField
            label={text.settings.voice}
            // Voix mémorisée disparue : choix vide, pour que choisir la voix de repli l'enregistre.
            value={gone ? "" : (used ?? "")}
            options={[
              ...(gone ? [{ value: "", label: text.settings.goneOption }] : []),
              ...voices.map((voice) => ({
                value: voice.id,
                label: text.settings.voiceOption(voice.name, voice.language),
              })),
            ]}
            onChange={(event) => {
              if (event.target.value === "") return;
              preview.stop();
              onChange(event.target.value);
            }}
          />
          <div>
            <Button
              variant="secondary"
              size="sm"
              icon={preview.playing ? "close" : "play"}
              onClick={() =>
                preview.playing
                  ? preview.stop()
                  : preview.play(text.settings.trySample, used)
              }
            >
              {preview.playing ? text.stop : text.settings.tryLabel}
            </Button>
          </div>
          <p role="status" className="fl-visually-hidden">
            {preview.playing ? text.playing : ""}
          </p>
          {preview.error && (
            <p role="alert" className="fl-field__error">
              {previewErrorMessage(preview.error)}
            </p>
          )}
        </>
      )}
    </section>
  );
}

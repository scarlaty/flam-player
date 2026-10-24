import { useEffect, useId, useState } from "react";
import { getSettings } from "../../../api/settings";
import { effectiveVoice } from "../../../api/speech";
import { Button, CheckboxField } from "../../../components";
import { t } from "../../../i18n";
import { previewErrorMessage, useSpeechPreview, useVoices } from "../../speech/speech";

export interface TitlesAudioFieldProps {
  /** Case « Lire les titres à voix haute » (`titles_audio` de la composition). */
  checked: boolean;
  onChange: (checked: boolean) => void;
  /** Voix fixée par la composition (`titles_voice`) ; absente : voix des Paramètres. */
  voice: string | null;
  /** Titre du pack lu par l'aperçu ; vide : rien à écouter. */
  previewText: string;
}

/**
 * Titres lus à voix haute (#462, note § 12.7) : case, voix utilisée et aperçu du titre du pack.
 * Sans voix française (ou synthèse indisponible), la case est désactivée avec une explication ;
 * elle reste décochable si un brouillon l'avait cochée.
 */
export function TitlesAudioField({
  checked,
  onChange,
  voice,
  previewText,
}: TitlesAudioFieldProps) {
  const text = t.speech;
  const id = useId();
  const { list, error: listError } = useVoices();
  const preview = useSpeechPreview();
  const [preferred, setPreferred] = useState<string | null>(null);
  useEffect(() => {
    let cancelled = false;
    getSettings()
      .then((view) => {
        if (!cancelled) setPreferred(view.settings.tts_voice ?? null);
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, []);

  const { stop } = preview;
  const trimmed = previewText.trim();
  // Un aperçu dont le texte change est coupé (note § 12.7).
  useEffect(() => stop, [trimmed, stop]);

  const loading = list === null && listError === null;
  const voices = list?.voices ?? [];
  const hasVoice = voices.length > 0;
  const wanted = voice ?? preferred;
  const used = list ? effectiveVoice(list, wanted) : null;
  const usedName = voices.find((v) => v.id === used)?.name ?? null;
  const gone = wanted !== null && hasVoice && used !== wanted;

  const statusId = `${id}-status`;
  let status: string;
  if (loading) status = text.loading;
  else if (listError) status = text.listFailed;
  else if (!hasVoice) status = text.noVoice;
  else if (gone) status = text.voiceGone(usedName ?? "");
  else if (voice !== null) status = text.voiceOfComposition(usedName ?? "");
  else status = text.voiceUsed(usedName ?? "");

  return (
    <div className="fl-pack__speech">
      <CheckboxField
        label={text.checkbox}
        hint={text.checkboxHint}
        describedBy={statusId}
        checked={checked}
        // Toujours décochable : un brouillon coché reste corrigeable sans voix.
        disabled={!checked && !hasVoice}
        onChange={(event) => onChange(event.target.checked)}
      />
      <p id={statusId} className="fl-pack__hint" aria-live="polite">
        {status}
      </p>
      {hasVoice && (
        <div className="fl-pack__speech-preview">
          <Button
            variant="secondary"
            size="sm"
            icon={preview.playing ? "close" : "play"}
            disabled={trimmed === "" && !preview.playing}
            aria-label={preview.playing ? text.stopTitle : text.listenTitle}
            aria-describedby={trimmed === "" ? `${id}-notitle` : undefined}
            onClick={() => (preview.playing ? stop() : preview.play(trimmed, used))}
          >
            {preview.playing ? text.stop : text.listen}
          </Button>
          {trimmed === "" && (
            <span id={`${id}-notitle`} className="fl-pack__hint">
              {text.noTitle}
            </span>
          )}
        </div>
      )}
      <p role="status" className="fl-visually-hidden">
        {preview.playing ? text.playing : ""}
      </p>
      {preview.error && (
        <p role="alert" className="fl-field__error">
          {previewErrorMessage(preview.error)}
        </p>
      )}
    </div>
  );
}

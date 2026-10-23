import { useRef, useState, type FormEvent } from "react";
import { addPodcast, type PodcastAdded } from "../../api/podcast";
import { toAppError, type AppError } from "../../api/error";
import { Banner, Button, TextField } from "../../components";
import { t } from "../../i18n";
import { validateCatalogUrl, validateName } from "../validation";
import { feedErrorTitle } from "./feedErrors";

/** Formulaire d'ajout d'un flux : validation locale, puis erreurs distinctes du backend. */
export function AddFeedForm({ onAdded }: { onAdded: (added: PodcastAdded) => void }) {
  const [url, setUrl] = useState("");
  const [name, setName] = useState("");
  const [touched, setTouched] = useState({ url: false, name: false });
  const [error, setError] = useState<AppError | null>(null);
  const [busy, setBusy] = useState(false);

  const urlInput = useRef<HTMLInputElement>(null);
  const nameInput = useRef<HTMLInputElement>(null);

  const nameValue = name.trim();
  const urlError = touched.url ? validateCatalogUrl(url) : null;
  const nameError = touched.name && nameValue !== "" ? validateName(name) : null;

  async function submit(event: FormEvent) {
    event.preventDefault();
    if (busy) return;
    setTouched({ url: true, name: true });
    const invalidUrl = validateCatalogUrl(url) !== null;
    const invalidName = nameValue !== "" && validateName(name) !== null;
    if (invalidUrl || invalidName) {
      // Focus sur le premier champ invalide.
      (invalidUrl ? urlInput : nameInput).current?.focus();
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const added = await addPodcast(
        url.trim(),
        nameValue === "" ? undefined : nameValue,
      );
      setUrl("");
      setName("");
      setTouched({ url: false, name: false });
      onAdded(added);
    } catch (caught) {
      setError(toAppError(caught));
    } finally {
      setBusy(false);
    }
  }

  const text = t.podcasts.add;
  return (
    <section className="fl-podcasts__section" aria-labelledby="fl-podcasts-add">
      <h2 id="fl-podcasts-add" className="fl-podcasts__heading">
        {text.title}
      </h2>
      <form className="fl-form" noValidate onSubmit={submit}>
        {error && (
          <Banner tone="danger" title={feedErrorTitle(error)}>
            <p>{error.message}</p>
            {feedErrorTitle(error) === t.podcasts.error.network && (
              <p>{t.podcasts.error.networkHint}</p>
            )}
          </Banner>
        )}
        <TextField
          label={text.url}
          hint={text.urlHint}
          type="url"
          inputMode="url"
          autoComplete="off"
          spellCheck={false}
          required
          ref={urlInput}
          value={url}
          error={urlError}
          readOnly={busy}
          onChange={(e) => {
            setUrl(e.target.value);
            setTouched((s) => ({ ...s, url: true }));
          }}
        />
        <TextField
          label={text.name}
          hint={text.nameHint}
          ref={nameInput}
          value={name}
          error={nameError}
          readOnly={busy}
          onChange={(e) => {
            setName(e.target.value);
            setTouched((s) => ({ ...s, name: true }));
          }}
        />
        <div className="fl-podcasts__actions">
          <Button variant="primary" type="submit" disabled={busy}>
            {text.submit}
          </Button>
          <p role="status" className="fl-podcasts__status">
            {busy ? text.adding : ""}
          </p>
        </div>
      </form>
    </section>
  );
}

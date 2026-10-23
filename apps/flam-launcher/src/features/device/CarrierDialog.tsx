import { useEffect, useRef, useState } from "react";
import {
  acceptCarrierWarning,
  BT_HEX_DIGITS,
  getCarrierState,
  normalizeBtHex,
  previewEnteredBt,
  type EnteredPreview,
  type CarrierChoice,
  type CarrierSend,
  type CarrierState,
} from "../../api/device";
import { pickFile } from "../../api/dialog";
import { toAppError } from "../../api/error";
import {
  Banner,
  Button,
  CheckboxField,
  Dialog,
  RadioGroup,
  TextField,
  type RadioOption,
} from "../../components";
import { t } from "../../i18n";

const text = t.deviceTransfer.carrier;

export interface CarrierDialogProps {
  /** Racine de la Flam (firmware 1.x). */
  root: string;
  onCancel: () => void;
  /** Source choisie, après consentement. Le backend revérifie tout avant la copie. */
  onConfirm: (carrier: CarrierSend) => void;
}

type Load =
  | { phase: "loading" }
  | { phase: "error"; message: string }
  | { phase: "ready"; state: CarrierState };

const STORED = "stored";
const FILE = "file";
const ENTERED = "entered";
const DEVICE_PREFIX = "device:";
/** Longueur maximale du champ du `bt` : 64 chiffres hexadécimaux et des espaces de groupement. */
const BT_MAX_LENGTH = 128;

function defaultSource(state: CarrierState): string {
  if (state.stored_keys) return STORED;
  const first = state.candidates[0];
  return first ? `${DEVICE_PREFIX}${first.uuid}` : ENTERED;
}

function choiceOf(
  source: string,
  file: string | null,
  bt: string,
  confirmed: boolean,
): CarrierChoice | null {
  if (source === STORED) return { kind: "stored" };
  if (source.startsWith(DEVICE_PREFIX))
    return { kind: "device", uuid: source.slice(DEVICE_PREFIX.length) };
  if (source === ENTERED) {
    const hex = normalizeBtHex(bt);
    return hex && confirmed ? { kind: "entered", bt: hex, confirmed: true } : null;
  }
  return file ? { kind: "file", path: file } : null;
}

/** Erreur de saisie du `bt` (aucune tant que le champ est vide). */
function btError(bt: string): string | null {
  const compact = bt.replace(/\s+/g, "");
  if (compact === "" || normalizeBtHex(compact)) return null;
  if (!/^[0-9a-fA-F]*$/.test(compact)) return text.enteredNotHex;
  return text.enteredLength(compact.length, BT_HEX_DIGITS);
}

function candidateHint(candidate: { hidden: boolean; with_stored: boolean }) {
  const hints = [
    candidate.hidden ? text.hiddenHint : null,
    candidate.with_stored ? text.withStoredHint : null,
  ].filter(Boolean);
  return hints.length > 0 ? hints.join(" · ") : undefined;
}

function formatDate(iso: string): string {
  const date = new Date(iso);
  return Number.isNaN(date.getTime()) ? iso : date.toLocaleString("fr-FR");
}

/**
 * Mode carrier (Flam en firmware 1.x) : avertissement à accepter (case « J'ai compris »
 * obligatoire) à la première utilisation, puis choix des clés (histoire officielle de la Flam,
 * clés conservées, clé `bt` saisie ou fichier). Monté à chaque ouverture.
 */
export function CarrierDialog({ root, onCancel, onConfirm }: CarrierDialogProps) {
  const [load, setLoad] = useState<Load>({ phase: "loading" });
  const [reload, setReload] = useState(0);
  const [understood, setUnderstood] = useState(false);
  const [accepting, setAccepting] = useState(false);
  const [acceptError, setAcceptError] = useState<string | null>(null);
  const [source, setSource] = useState<string | null>(null);
  const [file, setFile] = useState<string | null>(null);
  const [bt, setBt] = useState("");
  const [showBt, setShowBt] = useState(false);
  // Histoire reconnue par la clé saisie (`hex` : saisie vérifiée) et confirmation du titre.
  const [preview, setPreview] = useState<{ hex: string; story: EnteredPreview } | null>(
    null,
  );
  const [checking, setChecking] = useState(false);
  const [checkError, setCheckError] = useState<string | null>(null);
  const [titleConfirmed, setTitleConfirmed] = useState(false);
  const [remember, setRemember] = useState(true);
  const cancelButton = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    let active = true;
    getCarrierState(root)
      .then((state) => {
        if (active) setLoad({ phase: "ready", state });
      })
      .catch((caught) => {
        if (active) setLoad({ phase: "error", message: toAppError(caught).message });
      });
    return () => {
      active = false;
    };
  }, [root, reload]);

  const state = load.phase === "ready" ? load.state : null;
  const consented = state?.consent ?? null;

  async function accept() {
    if (!state || !understood || accepting) return;
    setAccepting(true);
    setAcceptError(null);
    try {
      await acceptCarrierWarning(state.warning_version, true);
      setLoad({ phase: "loading" });
      setReload((n) => n + 1);
    } catch (caught) {
      setAcceptError(toAppError(caught).message);
    } finally {
      setAccepting(false);
    }
  }

  async function browse() {
    try {
      const picked = await pickFile(text.pickTitle);
      if (picked) setFile(picked);
    } catch (caught) {
      setAcceptError(toAppError(caught).message);
    }
  }

  const current = state ? (source ?? defaultSource(state)) : FILE;
  const enteredHex = normalizeBtHex(bt);
  const recognized = preview && preview.hex === enteredHex ? preview.story : null;
  const choice = choiceOf(current, file, bt, recognized !== null && titleConfirmed);

  function changeBt(value: string) {
    setBt(value);
    setPreview(null);
    setTitleConfirmed(false);
    setCheckError(null);
  }

  async function checkBt() {
    if (!enteredHex || checking) return;
    setChecking(true);
    setCheckError(null);
    try {
      const story = await previewEnteredBt(root, enteredHex);
      setPreview({ hex: enteredHex, story });
      setTitleConfirmed(false);
    } catch (caught) {
      setCheckError(toAppError(caught).message);
    } finally {
      setChecking(false);
    }
  }
  const options: RadioOption<string>[] = state
    ? [
        ...(state.stored_keys ? [{ value: STORED, label: text.stored }] : []),
        ...state.candidates.map((candidate) => ({
          value: `${DEVICE_PREFIX}${candidate.uuid}`,
          label: text.fromStory(candidate.title),
          hint: candidateHint(candidate),
        })),
        { value: ENTERED, label: text.fromEntered, hint: text.fromEnteredHint },
        { value: FILE, label: text.fromFile, hint: text.fromFileHint },
      ]
    : [];

  const footer =
    state && !consented ? (
      <>
        <Button ref={cancelButton} onClick={onCancel}>
          {t.common.cancel}
        </Button>
        <Button
          variant="primary"
          disabled={!understood || accepting}
          onClick={() => void accept()}
        >
          {accepting ? text.accepting : text.continue}
        </Button>
      </>
    ) : (
      <>
        <Button ref={cancelButton} onClick={onCancel}>
          {t.common.cancel}
        </Button>
        {state && (
          <Button
            variant="primary"
            icon="install"
            disabled={choice === null}
            onClick={() => {
              if (choice)
                onConfirm({ choice, remember: choice.kind !== "stored" && remember });
            }}
          >
            {text.confirm}
          </Button>
        )}
      </>
    );

  return (
    <Dialog
      open
      onClose={onCancel}
      role="alertdialog"
      title={text.title}
      initialFocus={cancelButton}
      footer={footer}
    >
      {load.phase === "loading" && <p role="status">{text.loading}</p>}
      {load.phase === "error" && (
        <Banner tone="danger" role="alert">
          {text.loadFailed(load.message)}
        </Banner>
      )}
      {acceptError && (
        <Banner tone="danger" role="alert">
          {acceptError}
        </Banner>
      )}
      {state && !consented && (
        <>
          <h3>{text.warningTitle}</h3>
          <Banner tone="warning">{state.warning_text}</Banner>
          <CheckboxField
            label={text.understood}
            checked={understood}
            onChange={(event) => setUnderstood(event.target.checked)}
          />
        </>
      )}
      {state && consented && (
        <>
          <p>{text.acceptedAt(formatDate(consented.accepted_at))}</p>
          {state.candidates.length === 0 && !state.stored_keys && (
            <Banner tone="info">{text.noCandidate}</Banner>
          )}
          <RadioGroup
            legend={text.sourceLegend}
            options={options}
            value={current}
            onChange={setSource}
          />
          {current === ENTERED && (
            <>
              <TextField
                label={text.enteredLabel}
                hint={text.enteredHint}
                type={showBt ? "text" : "password"}
                autoComplete="off"
                autoCapitalize="off"
                spellCheck={false}
                maxLength={BT_MAX_LENGTH}
                value={bt}
                error={btError(bt)}
                onChange={(event) => changeBt(event.target.value)}
              />
              <CheckboxField
                label={text.showEntered}
                checked={showBt}
                onChange={(event) => setShowBt(event.target.checked)}
              />
              <p>
                <Button disabled={!enteredHex || checking} onClick={() => void checkBt()}>
                  {checking ? text.checkingEntered : text.checkEntered}
                </Button>
              </p>
              {checkError && (
                <Banner tone="danger" role="alert">
                  {checkError}
                </Banner>
              )}
              {recognized && (
                <>
                  <Banner tone="warning" role="status">
                    {text.recognized(recognized.title)}
                  </Banner>
                  <CheckboxField
                    label={text.titleConfirmed}
                    checked={titleConfirmed}
                    onChange={(event) => setTitleConfirmed(event.target.checked)}
                  />
                </>
              )}
            </>
          )}
          {current === FILE && (
            <p>
              <Button onClick={() => void browse()}>{text.pick}</Button>{" "}
              {file && <span>{text.picked(file)}</span>}
            </p>
          )}
          {current !== STORED && (
            <CheckboxField
              label={text.remember}
              hint={text.rememberHint}
              checked={remember}
              onChange={(event) => setRemember(event.target.checked)}
            />
          )}
        </>
      )}
    </Dialog>
  );
}

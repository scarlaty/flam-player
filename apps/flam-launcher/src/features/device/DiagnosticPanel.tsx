import { useEffect, useId, useRef, useState } from "react";
import { getDeviceDiagnostic } from "../../api/device";
import { toAppError } from "../../api/error";
import { openExternal } from "../../api/external";
import { Banner, Button } from "../../components";
import { t } from "../../i18n";
import "./DiagnosticPanel.css";

const text = t.deviceDiagnostic;

/** Modèle d'issue « Retour testeur Flam » (`.github/ISSUE_TEMPLATE/retour-testeur-flam.yml`). */
export const REPORT_ISSUE_URL =
  "https://github.com/scarlaty/flam-player/issues/new?template=retour-testeur-flam.yml";

type LinkNote = "idle" | "copied" | "copyFailed" | "openFailed";

const LINK_NOTES: Record<LinkNote, string> = {
  idle: "",
  copied: text.linkCopied,
  copyFailed: text.linkCopyFailed,
  openFailed: text.openFailed,
};

type State =
  | { phase: "idle" }
  | { phase: "working" }
  | { phase: "copied" }
  | { phase: "fallback"; diagnostic: string }
  | { phase: "error"; message: string };

/**
 * « Copier le diagnostic » (#318) : texte Markdown à coller dans une issue « Retour testeur
 * Flam ». Toujours affiché, même sans Flam détectée (diagnostic réduit) et quand l'écriture est
 * bloquée. Si le presse-papiers refuse, le texte est montré dans une zone sélectionnable.
 */
export function DiagnosticPanel() {
  const [state, setState] = useState<State>({ phase: "idle" });
  const [linkNote, setLinkNote] = useState<LinkNote>("idle");
  const areaRef = useRef<HTMLTextAreaElement>(null);
  const titleId = useId();
  const areaId = useId();
  const working = state.phase === "working";

  // Repli : le texte est sélectionné pour une copie au clavier.
  useEffect(() => {
    if (state.phase !== "fallback") return;
    areaRef.current?.focus();
    areaRef.current?.select();
  }, [state]);

  async function copy() {
    if (working) return;
    setState({ phase: "working" });
    let diagnostic: string;
    try {
      diagnostic = await getDeviceDiagnostic();
    } catch (error) {
      setState({ phase: "error", message: toAppError(error).message });
      return;
    }
    try {
      await navigator.clipboard.writeText(diagnostic);
      setState({ phase: "copied" });
    } catch {
      setState({ phase: "fallback", diagnostic });
    }
  }

  async function openReport() {
    setLinkNote("idle");
    try {
      await openExternal(REPORT_ISSUE_URL);
    } catch {
      setLinkNote("openFailed");
    }
  }

  async function copyLink() {
    setLinkNote("idle");
    try {
      await navigator.clipboard.writeText(REPORT_ISSUE_URL);
      setLinkNote("copied");
    } catch {
      setLinkNote("copyFailed");
    }
  }

  const announcement =
    state.phase === "copied"
      ? text.copied
      : state.phase === "fallback"
        ? text.copyFailed
        : "";

  return (
    <section className="fl-device-diagnostic" aria-labelledby={titleId}>
      <h2 id={titleId} className="fl-device-card__title">
        {text.title}
      </h2>
      <p className="fl-device-diagnostic__intro">{text.intro}</p>
      <div className="fl-device-diagnostic__actions">
        <Button
          aria-disabled={working || undefined}
          aria-busy={working || undefined}
          onClick={() => void copy()}
        >
          {working ? text.working : text.copy}
        </Button>
        <Button onClick={() => void openReport()}>{text.report}</Button>
        <Button variant="ghost" onClick={() => void copyLink()}>
          {text.copyLink}
        </Button>
      </div>
      <p className="fl-device-diagnostic__intro">
        {text.reportHint}{" "}
        <span className="fl-device-diagnostic__url" lang="en">
          {REPORT_ISSUE_URL}
        </span>
      </p>
      <p aria-live="polite" className="fl-device-diagnostic__note">
        {LINK_NOTES[linkNote]}
      </p>
      <p aria-live="polite" className="fl-device-diagnostic__note">
        {announcement}
      </p>
      {state.phase === "error" && (
        <Banner tone="danger" role="alert">
          {text.failed(state.message)}
        </Banner>
      )}
      {state.phase === "fallback" && (
        <div className="fl-device-diagnostic__fallback">
          <label htmlFor={areaId}>{text.fieldLabel}</label>
          <textarea
            id={areaId}
            ref={areaRef}
            readOnly
            rows={12}
            spellCheck={false}
            value={state.diagnostic}
          />
        </div>
      )}
    </section>
  );
}

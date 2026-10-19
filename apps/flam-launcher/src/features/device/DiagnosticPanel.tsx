import { useEffect, useId, useRef, useState } from "react";
import { getDeviceDiagnostic } from "../../api/device";
import { toAppError } from "../../api/error";
import { Banner, Button } from "../../components";
import { t } from "../../i18n";
import "./DiagnosticPanel.css";

const text = t.deviceDiagnostic;

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
      <div>
        <Button
          aria-disabled={working || undefined}
          aria-busy={working || undefined}
          onClick={() => void copy()}
        >
          {working ? text.working : text.copy}
        </Button>
      </div>
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

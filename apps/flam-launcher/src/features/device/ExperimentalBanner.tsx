import { useState } from "react";
import { Banner, Button } from "../../components";
import type { FirmwareFamily } from "../../api/device";
import { t } from "../../i18n";

const text = t.deviceBeta;

/**
 * Page des risques (documentation du dépôt). Le launcher n'a pas de commande pour ouvrir le
 * navigateur : l'adresse est affichée en clair et se copie en un clic.
 */
export const RISKS_DOC_URL =
  "https://github.com/scarlaty/flam-player/blob/main/apps/flam-launcher/docs/flam-transfert.md#risques";

/**
 * Bandeau de l'écran « Ma Flam » : fonction expérimentale et non officielle, avec renvoi vers
 * les risques. Pour une Flam en firmware 2.x (famille de l'état du firmware, #317), il ajoute que
 * ce firmware est en test.
 */
export function ExperimentalBanner({ family }: { family: FirmwareFamily | null }) {
  const [copied, setCopied] = useState<"idle" | "done" | "failed">("idle");

  async function copy() {
    setCopied("idle");
    try {
      await navigator.clipboard.writeText(RISKS_DOC_URL);
      setCopied("done");
    } catch {
      setCopied("failed");
    }
  }

  return (
    <Banner
      tone="warning"
      title={text.title}
      role="none"
      action={
        <Button variant="secondary" size="sm" onClick={() => void copy()}>
          {text.copy}
        </Button>
      }
    >
      <p className="fl-device-beta__line">
        {text.risks}{" "}
        <span className="fl-device-beta__url" lang="en">
          {RISKS_DOC_URL}
        </span>
      </p>
      {family === "v2" && <p className="fl-device-beta__line">{text.firmware2}</p>}
      <p
        aria-live="polite"
        className={copied === "idle" ? undefined : "fl-device-beta__line"}
      >
        {copied === "done" ? text.copied : copied === "failed" ? text.copyFailed : ""}
      </p>
    </Banner>
  );
}

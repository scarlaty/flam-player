import { useState } from "react";
import { openExternal } from "../../api/external";
import { Banner, Button } from "../../components";
import type { FirmwareFamily } from "../../api/device";
import { t } from "../../i18n";
import "./ExperimentalBanner.css";

const text = t.deviceBeta;

/**
 * Page des risques (documentation du dépôt). Elle s'ouvre dans le navigateur ; l'adresse reste
 * affichée en clair et se copie en un clic, en repli si l'ouverture échoue.
 */
export const RISKS_DOC_URL =
  "https://github.com/scarlaty/flam-player/blob/main/apps/flam-launcher/docs/flam-transfert.md#risques";

/**
 * Bandeau de l'écran « Ma Flam » : fonction expérimentale et non officielle, avec renvoi vers
 * les risques. Pour une Flam en firmware 2.x (famille de l'état du firmware, #317), il ajoute que
 * ce firmware est en test.
 */
export function ExperimentalBanner({ family }: { family: FirmwareFamily | null }) {
  const [copied, setCopied] = useState<"idle" | "done" | "failed" | "openFailed">("idle");

  async function open() {
    setCopied("idle");
    try {
      await openExternal(RISKS_DOC_URL);
    } catch {
      setCopied("openFailed");
    }
  }

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
        <div className="fl-device-beta__actions">
          <Button variant="secondary" size="sm" onClick={() => void open()}>
            {text.open}
          </Button>
          <Button variant="secondary" size="sm" onClick={() => void copy()}>
            {text.copy}
          </Button>
        </div>
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
        {copied === "done"
          ? text.copied
          : copied === "failed"
            ? text.copyFailed
            : copied === "openFailed"
              ? text.openFailed
              : ""}
      </p>
    </Banner>
  );
}

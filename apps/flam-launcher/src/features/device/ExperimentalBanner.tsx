import { useState } from "react";
import { Banner, Button } from "../../components";
import { t } from "../../i18n";

const text = t.deviceBeta;

/**
 * Page des risques (documentation du dépôt). Le launcher n'a pas de commande pour ouvrir le
 * navigateur : l'adresse est affichée en clair et se copie en un clic.
 */
export const RISKS_DOC_URL =
  "https://github.com/scarlaty/flam-player/blob/main/apps/flam-launcher/docs/flam-transfert.md#risques";

/** Firmware 2.x : première composante exactement « 2 » (même règle que le backend). */
export function isFirmware2(firmware: string | null | undefined): boolean {
  return firmware?.split(".")[0] === "2";
}

/**
 * Bandeau de l'écran « Ma Flam » : fonction expérimentale et non officielle, avec renvoi vers
 * les risques. Pour une Flam en firmware 2.x, il ajoute que ce firmware est en test.
 */
export function ExperimentalBanner({ firmware }: { firmware: string | null }) {
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
      {isFirmware2(firmware) && <p className="fl-device-beta__line">{text.firmware2}</p>}
      <p
        aria-live="polite"
        className={copied === "idle" ? undefined : "fl-device-beta__line"}
      >
        {copied === "done" ? text.copied : copied === "failed" ? text.copyFailed : ""}
      </p>
    </Banner>
  );
}

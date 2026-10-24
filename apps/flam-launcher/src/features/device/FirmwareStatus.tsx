import {
  firmwareWriteBlock,
  type DeviceSummary,
  type FirmwareStatus,
  type WriteRefused,
} from "../../api/device";
import { Banner } from "../../components";
import { t } from "../../i18n";

const text = t.deviceFirmware;

/** Message lisible d'un refus d'écrire de la garde du firmware. */
export function blockedMessage(block: WriteRefused): string {
  const messages = text.blocked;
  switch (block.kind) {
    case "device_not_found":
    case "invalid_root":
    case "version_unreadable":
      return messages[block.kind];
    case "mdf_unreadable":
    case "keys_unreadable":
      return messages[block.kind](block.detail);
    case "truncated":
      return messages.truncated(block.size);
    case "unsupported_firmware":
    case "keys_trivial":
      return messages[block.kind](block.version);
  }
}

/** Ligne courte « 2.4.2 (2.x) · pris en charge » de la carte « Flam détectée ». */
export function firmwareSummary(status: FirmwareStatus): string {
  const version = status.version ?? text.unreadable;
  const family = status.family === "unreadable" ? null : text.family[status.family];
  const head = family && status.version ? `${version} (${family})` : version;
  const parts = [head, firmwareState(status)];
  const known = status.family === "v1" || status.family === "v2";
  if (known && !status.layout_standard) parts.push(text.nonStandard);
  return parts.join(" · ");
}

function firmwareState(status: FirmwareStatus): string {
  if (status.write_block) return text.state.blocked;
  switch (status.family) {
    case "v1":
      return text.state.v1;
    case "v2":
      return status.coherence.kind === "contradiction"
        ? text.state.contradiction
        : text.state.v2;
    default:
      return text.state.blocked;
  }
}

/**
 * Bandeaux de l'état du firmware : écriture bloquée (raison et version), ou avertissement de
 * clés incohérentes (#326 : l'envoi reste possible, faux positifs possibles).
 */
export function FirmwareNotice({ device }: { device: DeviceSummary }) {
  const block = firmwareWriteBlock(device);
  const status = device.firmware_status;
  const coherence = status?.coherence;
  const contradiction = !block && coherence?.kind === "contradiction" ? coherence : null;
  // `.mdf` 2.x d'une forme jamais mesurée : envoi possible, risque accru d'histoire illisible.
  const nonStandard = !block && status?.family === "v2" && !status.layout_standard;
  return (
    <>
      {block && (
        <Banner tone="danger" role="status" title={text.blockedTitle}>
          <p>{blockedMessage(block)}</p>
          <p>{text.blockedHint}</p>
        </Banner>
      )}
      {nonStandard && (
        <Banner tone="warning" role="status" title={text.nonStandardTitle}>
          <p>{text.nonStandardBody(status.mdf_size, status.header ?? "?")}</p>
        </Banner>
      )}
      {contradiction && (
        <Banner tone="warning" role="status" title={text.contradictionTitle}>
          <p>{text.contradiction(status.version ?? text.unreadable)}</p>
          {contradiction.stories.length > 0 && (
            <p style={{ overflowWrap: "anywhere" }}>
              {text.contradictionStories(contradiction.stories, contradiction.total)}
            </p>
          )}
          <p>{text.contradictionHint}</p>
        </Banner>
      )}
    </>
  );
}

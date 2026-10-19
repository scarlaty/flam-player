import { useState } from "react";
import {
  confirmFirmware,
  firmwareWriteBlock,
  needsFirmwareConfirmation,
  type DeviceSummary,
  type FirmwareStatus,
  type WriteRefused,
} from "../../api/device";
import { toAppError } from "../../api/error";
import { Banner, Button, CheckboxField } from "../../components";
import { t } from "../../i18n";
import { useDevice } from "./DeviceContext";

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
    case "keys_contradiction":
    case "confirmation_required":
      return messages[block.kind](block.version);
  }
}

/** Ligne courte « 2.4.2 (2.x) · en test, vérifié par la Flam » de la carte « Flam détectée ». */
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
  const block = status.write_block;
  if (block?.kind === "keys_contradiction") return text.state.contradiction;
  if (block && block.kind !== "confirmation_required") return text.state.blocked;
  switch (status.trust) {
    case "not_applicable":
      return text.state.v1;
    case "flam_verified":
      return text.state.flamVerified;
    case "confirmed":
      return text.state.confirmed;
    case "unconfirmed":
      return text.state.unconfirmed;
  }
}

/**
 * Bandeaux de l'état du firmware : écriture ou envoi bloqués (raison et version), ou
 * confirmation de sauvegarde demandée pour une Flam 2.x.
 */
export function FirmwareNotice({ device }: { device: DeviceSummary }) {
  const block = firmwareWriteBlock(device);
  if (block) {
    const title =
      block.kind === "keys_contradiction" ? text.sendBlockedTitle : text.blockedTitle;
    return (
      <Banner tone="danger" role="status" title={title}>
        <p>{blockedMessage(block)}</p>
        <p>{text.blockedHint}</p>
      </Banner>
    );
  }
  if (needsFirmwareConfirmation(device)) return <ConfirmFirmware device={device} />;
  return null;
}

function ConfirmFirmware({ device }: { device: DeviceSummary }) {
  const { retry } = useDevice();
  const [understood, setUnderstood] = useState(false);
  const [working, setWorking] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);
  const status = device.firmware_status;

  async function confirm() {
    setWorking(true);
    setFailure(null);
    try {
      await confirmFirmware(device.root, true);
      retry();
    } catch (caught) {
      setFailure(text.confirm.failed(toAppError(caught).message));
    } finally {
      setWorking(false);
    }
  }

  return (
    <Banner
      tone={status.layout_standard ? "warning" : "danger"}
      role="status"
      title={text.confirm.title}
    >
      <p>{text.confirm.body}</p>
      {!status.layout_standard && (
        <p>{text.confirm.nonStandard(status.mdf_size, status.header ?? "?")}</p>
      )}
      <CheckboxField
        label={text.confirm.understood}
        checked={understood}
        onChange={(event) => setUnderstood(event.target.checked)}
      />
      <Button
        variant="primary"
        disabled={!understood || working}
        onClick={() => void confirm()}
      >
        {working ? text.confirm.working : text.confirm.action}
      </Button>
      {failure && <p role="alert">{failure}</p>}
    </Banner>
  );
}

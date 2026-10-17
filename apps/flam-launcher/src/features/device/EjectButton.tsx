import { useId } from "react";
import { Button } from "../../components";
import { t } from "../../i18n";
import { useDevice } from "./DeviceContext";
import "./SendToDeviceButton.css";

const text = t.deviceTransfer.eject;

/** « Éjecter la Flam » ; refusé pendant un envoi (raison écrite à côté, bouton focalisable). */
export function EjectButton() {
  const { device, busy, ejecting, eject } = useDevice();
  const hintId = useId();
  if (!device) return null;
  const blocked = busy || ejecting;
  return (
    <span className="fl-send">
      <Button
        icon="device"
        className="fl-send__button"
        aria-disabled={blocked || undefined}
        aria-describedby={busy ? hintId : undefined}
        onClick={() => {
          if (!blocked) void eject();
        }}
      >
        {ejecting ? text.working : text.button}
      </Button>
      {busy && (
        <span id={hintId} className="fl-send__reason">
          {text.busy}
        </span>
      )}
    </span>
  );
}

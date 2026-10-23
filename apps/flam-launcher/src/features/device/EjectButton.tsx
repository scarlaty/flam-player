import { useId } from "react";
import { Button } from "../../components";
import { cx } from "../../components/cx";
import { t } from "../../i18n";
import { useDevice } from "./DeviceContext";
import "./SendToDeviceButton.css";

const text = t.deviceTransfer.eject;

/** « Éjecter la Flam » ; refusé pendant un envoi (raison écrite à côté, bouton focalisable). */
export function EjectButton() {
  const { device, busy, ejecting, removing, eject } = useDevice();
  const hintId = useId();
  if (!device) return null;
  const blocked = busy || ejecting || removing;
  return (
    <span className="fl-send">
      <Button
        icon={ejecting ? "refresh" : "device"}
        className={cx("fl-send__button", ejecting && "fl-send__button--working")}
        aria-disabled={blocked || undefined}
        aria-busy={ejecting || undefined}
        aria-describedby={busy || removing ? hintId : undefined}
        onClick={() => {
          if (!blocked) void eject();
        }}
      >
        {ejecting ? text.working : text.button}
      </Button>
      {(busy || removing) && (
        <span id={hintId} className="fl-send__reason">
          {busy ? text.busy : text.removing}
        </span>
      )}
    </span>
  );
}

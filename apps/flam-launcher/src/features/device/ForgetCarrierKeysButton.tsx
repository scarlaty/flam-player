import { useState } from "react";
import { needsCarrier } from "../../api/device";
import { toAppError } from "../../api/error";
import { Banner, Button } from "../../components";
import { t } from "../../i18n";
import { useDevice } from "./DeviceContext";
import "./SendToDeviceButton.css";

const text = t.deviceTransfer.carrier;

/** « Oublier les clés » : supprime du PC les clés carrier conservées pour la Flam 1.x branchée. */
export function ForgetCarrierKeysButton() {
  const { device, busy, forgetCarrier } = useDevice();
  const [working, setWorking] = useState(false);
  const [note, setNote] = useState<{
    tone: "success" | "danger";
    message: string;
  } | null>(null);
  if (!needsCarrier(device)) return null;

  async function forget() {
    if (working || busy) return;
    setWorking(true);
    setNote(null);
    try {
      const removed = await forgetCarrier();
      setNote({
        tone: "success",
        message: removed > 0 ? text.forgotten : text.nothingToForget,
      });
    } catch (caught) {
      setNote({ tone: "danger", message: text.forgetFailed(toAppError(caught).message) });
    } finally {
      setWorking(false);
    }
  }

  return (
    <span className="fl-send">
      <Button
        className="fl-send__button"
        aria-disabled={working || busy || undefined}
        onClick={() => void forget()}
      >
        {working ? text.forgetting : text.forget}
      </Button>
      {note && <Banner tone={note.tone}>{note.message}</Banner>}
    </span>
  );
}

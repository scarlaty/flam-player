import { t } from "../../i18n";
import { SendToDeviceButton } from "../device/SendToDeviceButton";
import { formatBytes } from "../validation";
import { useStories } from "./StoriesContext";

const text = t.deviceTransfer.fromLibrary;

/**
 * Section de « Ma Flam » : les histoires de « Mes histoires », chacune avec « Envoyer sur la
 * Flam » (bouton désactivé avec sa raison si l'envoi est impossible).
 */
export function DeviceLibrarySend() {
  const { installed } = useStories();
  return (
    <section className="fl-device-stories" aria-labelledby="fl-device-send-title">
      <h2 id="fl-device-send-title" className="fl-device-card__title">
        {text.title}
      </h2>
      {installed.length === 0 ? (
        <p>{text.empty}</p>
      ) : (
        <ul className="fl-device-rows" aria-label={text.listLabel}>
          {installed.map((entry) => (
            <li key={entry.uuid} className="fl-device-row">
              <div className="fl-device-row__text">
                <span className="fl-device-row__name">{entry.title}</span>
                <span className="fl-device-row__detail">{formatBytes(entry.size)}</span>
              </div>
              <div className="fl-device-row__actions">
                <SendToDeviceButton
                  uuid={entry.uuid}
                  title={entry.title}
                  size={entry.size}
                />
              </div>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

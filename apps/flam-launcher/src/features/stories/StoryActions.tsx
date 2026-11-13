import { useEffect, useId, useRef, useState } from "react";
import type { StoryEntry } from "../../api/story";
import { Button, IconButton } from "../../components";
import { t } from "../../i18n";
import { SendToDeviceButton } from "../device/SendToDeviceButton";
import { isRemote, useStories } from "./StoriesContext";
import { RemoveStoryDialog } from "./RemoveStoryDialog";
import { useInstallAndSend } from "./useInstallAndSend";

/**
 * Actions rapides d'une carte : « Jouer », « Envoyer sur la Flam » et « Supprimer » (bouton icône
 * compact, libellé et info-bulle) si l'histoire est installée, sinon « Télécharger » ou
 * « Installer », plus « Installer et envoyer » quand l'envoi sur la Flam branchée est possible ;
 * rien pendant qu'une tâche est active (la carte montre sa progression). Une
 * histoire STUdio installée (format Lunii) ne se joue pas sur le PC : « Jouer » reste visible,
 * désactivé, avec sa raison.
 */
export function StoryActions({ entry }: { entry: StoryEntry }) {
  const stories = useStories();
  const text = t.stories.actions;
  const [confirmRemove, setConfirmRemove] = useState(false);
  const [removing, setRemoving] = useState(false);
  const installRef = useRef<HTMLButtonElement>(null);
  const removeReasonId = useId();
  const playReasonId = useId();
  const sendHelpId = useId();
  const installAndSend = useInstallAndSend(entry);
  // Après une suppression réussie, le bouton disparaît : le focus passe au bouton de
  // réinstallation dès qu'il est affiché.
  const focusAfterRemove = useRef(false);
  const installed = stories.installedFor(entry);
  useEffect(() => {
    if (focusAfterRemove.current && !installed) {
      focusAfterRemove.current = false;
      installRef.current?.focus();
    }
  }, [installed]);
  if (stories.activeTaskFor(entry)) return null;
  if (installed) {
    const { session } = stories;
    const playingThis =
      session !== null &&
      session.uuid === (session.origin === "library" ? entry.uuid : entry.entry_id);
    const lunii = installed.format === "lunii";
    return (
      <>
        <Button
          size="sm"
          variant="accent"
          icon="play"
          disabled={stories.session !== null || lunii}
          title={lunii ? text.playUnavailable : undefined}
          aria-label={text.forStory(text.play, entry.title)}
          aria-describedby={lunii ? playReasonId : undefined}
          onClick={() => stories.play(entry)}
        >
          {text.play}
        </Button>
        {lunii && (
          <span id={playReasonId} className="fl-visually-hidden">
            {text.playUnavailable}
          </span>
        )}
        <SendToDeviceButton
          uuid={installed.uuid}
          title={entry.title}
          size={installed.size}
          format={installed.format}
          stale={stories.sourceChanged(installed.uuid)}
        />
        <IconButton
          size="sm"
          variant="danger-quiet"
          icon="trash"
          // `aria-disabled` : le bouton garde le focus (fermeture du dialogue) et sa raison est lue.
          aria-disabled={playingThis || removing || undefined}
          aria-busy={removing || undefined}
          aria-describedby={playingThis || removing ? removeReasonId : undefined}
          label={text.forStory(text.remove, entry.title)}
          onClick={() => {
            if (!playingThis && !removing) setConfirmRemove(true);
          }}
        />
        {(playingThis || removing) && (
          <span id={removeReasonId} className="fl-visually-hidden">
            {removing ? text.removing : text.removePlaying}
          </span>
        )}
        <RemoveStoryDialog
          open={confirmRemove}
          title={entry.title}
          onCancel={() => setConfirmRemove(false)}
          onConfirm={(purge) => {
            setConfirmRemove(false);
            setRemoving(true);
            focusAfterRemove.current = true;
            void stories
              .remove(installed.uuid, entry.title, purge)
              .then(
                (removed) => {
                  if (!removed) focusAfterRemove.current = false;
                },
                () => {
                  focusAfterRemove.current = false;
                },
              )
              .finally(() => setRemoving(false));
          }}
        />
      </>
    );
  }
  const remote = isRemote(entry);
  const label = remote ? text.download : text.install;
  const sendLabel = remote ? text.downloadAndSend : text.installAndSend;
  return (
    <>
      <Button
        ref={installRef}
        size="sm"
        icon="download"
        aria-label={text.forStory(label, entry.title)}
        onClick={() => stories.install(entry)}
      >
        {label}
      </Button>
      {installAndSend && (
        <>
          <Button
            size="sm"
            icon="device"
            title={text.installAndSendHelp}
            aria-label={text.forStory(sendLabel, entry.title)}
            aria-describedby={sendHelpId}
            onClick={installAndSend}
          >
            {sendLabel}
          </Button>
          <span id={sendHelpId} className="fl-visually-hidden">
            {text.installAndSendHelp}
          </span>
        </>
      )}
    </>
  );
}

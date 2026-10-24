import { useId, useState } from "react";
import type { Source } from "../../api/sources";
import type { ListedEntry } from "../library/sourceData";
import {
  Badge,
  Banner,
  Button,
  Dialog,
  Icon,
  ProgressBar,
  type BadgeTone,
} from "../../components";
import { placeholderTone } from "../../components/StoryCard/StoryCard";
import { t } from "../../i18n";
import { useCompat } from "../compat/CompatContext";
import { useDevice } from "../device/DeviceContext";
import { SendToDeviceButton } from "../device/SendToDeviceButton";
import { COMPAT_APPEARANCE, reasonText } from "../compat/compatInfo";
import { defaultStoryState, STATE_APPEARANCE } from "../library/storyState";
import { useThumbnail } from "../library/useThumbnail";
import { formatBytes } from "../validation";
import { isRemote, storyStateOf, useStories } from "./StoriesContext";
import { RemoveStoryDialog } from "./RemoveStoryDialog";
import { taskErrorMessage } from "./taskView";
import "./StoryDetail.css";

export interface StoryDetailProps {
  source: Source;
  entry: ListedEntry;
  onClose: () => void;
}

function Fact({ label, value }: { label: string; value: string | null | undefined }) {
  return (
    <div className="fl-detail__fact">
      <dt>{label}</dt>
      <dd>{value || t.stories.detail.unknown}</dd>
    </div>
  );
}

/**
 * Fiche d'une histoire : vignette, description, métadonnées, état, compatibilité et actions
 * (Télécharger et jouer, Jouer, Mettre à jour, Supprimer, Annuler). Les dialogues de
 * confirmation prennent la place de la fiche le temps de la réponse (un seul modal à la fois).
 */
export function StoryDetail({ source, entry, onClose }: StoryDetailProps) {
  const stories = useStories();
  const { confirmingSend } = useDevice();
  const { compatFor } = useCompat();
  const { url } = useThumbnail(source, entry, { eager: true });
  const [confirmRemove, setConfirmRemove] = useState(false);
  const [failedUrl, setFailedUrl] = useState<string>();
  const updateHelpId = useId();

  const state = storyStateOf(stories, entry) ?? defaultStoryState(source, entry);
  const compat = compatFor(entry);
  const task = stories.activeTaskFor(entry);
  const failed = stories.failedTaskFor(entry);
  const installed = stories.installedFor(entry);
  const progress = stories.progressFor(entry);
  const { session } = stories;
  const playing = session !== null;
  const playingThis =
    session !== null &&
    session.uuid === (session.origin === "library" ? entry.uuid : entry.entry_id);
  const canPlayFromFolder = source.type === "folder" && entry.kind !== "telmi_zip";
  const text = t.stories.actions;
  const fromCatalog = isRemote(entry);
  const showCover = url !== undefined && url !== failedUrl;
  const tone: BadgeTone = STATE_APPEARANCE[state].tone;

  async function confirmedRemove(purgeSaves: boolean) {
    const removed = await stories.remove(entry.uuid, entry.title, purgeSaves);
    setConfirmRemove(false);
    if (removed && source.type === "library") onClose();
  }

  return (
    <>
      <Dialog
        wide
        open={!confirmRemove && !stories.confirming && !confirmingSend}
        onClose={onClose}
        title={entry.title}
        footer={
          <div className="fl-detail__actions">
            {task && (
              <Button size="sm" icon="close" onClick={() => stories.cancel(task.id)}>
                {text.cancel}
              </Button>
            )}
            {!task && installed && (
              <>
                <Button
                  size="sm"
                  variant="danger-quiet"
                  icon="trash"
                  className="fl-detail__remove"
                  disabled={playingThis}
                  onClick={() => {
                    setConfirmRemove(true);
                  }}
                >
                  {text.remove}
                </Button>
                <SendToDeviceButton
                  uuid={installed.uuid}
                  title={entry.title}
                  size={installed.size}
                  stale={stories.sourceChanged(installed.uuid)}
                />
                {stories.updateAvailable(entry) && (
                  <>
                    <Button
                      size="sm"
                      icon="refresh"
                      title={text.updateHelp}
                      aria-describedby={updateHelpId}
                      onClick={() => stories.install(entry)}
                    >
                      {text.update}
                    </Button>
                    <span id={updateHelpId} className="fl-visually-hidden">
                      {text.updateHelp}
                    </span>
                  </>
                )}
                <Button
                  size="sm"
                  variant="accent"
                  icon="play"
                  disabled={playing}
                  onClick={() => stories.play(entry, source.id)}
                >
                  {text.play}
                </Button>
              </>
            )}
            {!task && !installed && (
              <>
                {canPlayFromFolder && (
                  <Button
                    size="sm"
                    icon="play"
                    disabled={playing}
                    onClick={() => stories.playFromFolder(entry, source.id)}
                  >
                    {text.playFromFolder}
                  </Button>
                )}
                <Button size="sm" icon="download" onClick={() => stories.install(entry)}>
                  {fromCatalog ? text.download : text.install}
                </Button>
                <Button
                  size="sm"
                  variant="accent"
                  icon="play"
                  disabled={playing}
                  onClick={() =>
                    stories.install(entry, { thenPlay: true, fromSourceId: source.id })
                  }
                >
                  {fromCatalog ? text.downloadAndPlay : text.installAndPlay}
                </Button>
              </>
            )}
          </div>
        }
      >
        <div className="fl-detail">
          <div className="fl-detail__cover">
            {showCover ? (
              <img
                src={url}
                alt={t.stories.detail.cover(entry.title)}
                onError={() => setFailedUrl(url)}
              />
            ) : (
              <div
                className={`fl-detail__placeholder fl-detail__placeholder--${placeholderTone(entry.title)}`}
                aria-hidden="true"
              >
                <Icon name="book" size={64} />
              </div>
            )}
          </div>
          <div className="fl-detail__body">
            <p className="fl-detail__description">
              {entry.description || t.stories.detail.noDescription}
            </p>
            <ul className="fl-detail__badges" aria-label={t.stories.detail.badges}>
              <li>
                <Badge
                  tone={tone}
                  icon={STATE_APPEARANCE[state].icon}
                  description={state === "update_available" ? text.updateHelp : undefined}
                >
                  {t.library.state[state]}
                </Badge>
              </li>
              {playingThis && (
                <li>
                  <Badge tone="secondary" icon="play">
                    {t.stories.playing}
                  </Badge>
                </li>
              )}
              {compat && (
                <li>
                  <Badge
                    tone={COMPAT_APPEARANCE[compat.level].tone}
                    icon={COMPAT_APPEARANCE[compat.level].icon}
                  >
                    {t.compat.level[compat.level]}
                  </Badge>
                </li>
              )}
            </ul>
            {compat && <p className="fl-detail__reason">{reasonText(compat)}</p>}
            <dl className="fl-detail__facts">
              <Fact
                label={t.stories.detail.facts.age}
                value={entry.age === null ? null : t.story.age(entry.age)}
              />
              <Fact label={t.stories.detail.facts.category} value={entry.category} />
              <Fact label={t.stories.detail.facts.author} value={entry.author} />
              <Fact label={t.stories.detail.facts.voice} value={entry.voice} />
              <Fact label={t.stories.detail.facts.license} value={entry.license} />
              <Fact
                label={t.stories.detail.facts.size}
                value={entry.size === null ? null : formatBytes(entry.size)}
              />
              <Fact label={t.stories.detail.facts.source} value={source.name} />
            </dl>
            {task && progress && (
              <div className="fl-detail__progress">
                <ProgressBar
                  value={progress.value === "indeterminate" ? undefined : progress.value}
                  label={t.story.progress(entry.title)}
                  showValue
                />
                <p aria-live="polite">{progress.text}</p>
              </div>
            )}
            {failed?.error && (
              <Banner
                tone="danger"
                action={
                  <Button onClick={() => stories.retry(failed.id)}>
                    {t.errors.retry}
                  </Button>
                }
              >
                <p>{taskErrorMessage(failed.error)}</p>
              </Banner>
            )}
            {playing && !playingThis && !task && (
              <p className="fl-detail__hint">{t.stories.playerBusy}</p>
            )}
          </div>
        </div>
      </Dialog>
      <RemoveStoryDialog
        open={confirmRemove}
        title={entry.title}
        onCancel={() => setConfirmRemove(false)}
        onConfirm={(purge) => void confirmedRemove(purge)}
      />
    </>
  );
}

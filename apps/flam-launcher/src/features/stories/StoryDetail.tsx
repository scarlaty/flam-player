import { useRef, useState } from "react";
import type { Source } from "../../api/sources";
import type { StoryEntry } from "../../api/story";
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
import { COMPAT_APPEARANCE, reasonText } from "../compat/compatInfo";
import { defaultStoryState, STATE_APPEARANCE } from "../library/storyState";
import { useThumbnail } from "../library/useThumbnail";
import { formatBytes } from "../validation";
import { isRemote, storyStateOf, useStories } from "./StoriesContext";
import "./StoryDetail.css";

export interface StoryDetailProps {
  source: Source;
  entry: StoryEntry;
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
  const { compatFor } = useCompat();
  const { url } = useThumbnail(source, entry);
  const [confirmRemove, setConfirmRemove] = useState(false);
  const cancelRef = useRef<HTMLButtonElement>(null);
  const [failedUrl, setFailedUrl] = useState<string>();

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

  async function confirmedRemove() {
    const removed = await stories.remove(entry.uuid, entry.title);
    setConfirmRemove(false);
    if (removed && source.type === "library") onClose();
  }

  return (
    <>
      <Dialog
        wide
        open={!confirmRemove && !stories.confirming}
        onClose={onClose}
        title={entry.title}
        footer={
          <>
            {task && (
              <Button icon="close" onClick={() => stories.cancel(task.id)}>
                {text.cancel}
              </Button>
            )}
            {!task && installed && (
              <>
                <Button
                  variant="danger"
                  icon="trash"
                  disabled={playingThis}
                  onClick={() => setConfirmRemove(true)}
                >
                  {text.remove}
                </Button>
                {stories.updateAvailable(entry) && (
                  <Button icon="refresh" onClick={() => stories.install(entry)}>
                    {text.update}
                  </Button>
                )}
                <Button
                  variant="accent"
                  icon="play"
                  disabled={playing}
                  onClick={() => stories.play(entry)}
                >
                  {text.play}
                </Button>
              </>
            )}
            {!task && !installed && (
              <>
                {canPlayFromFolder && (
                  <Button
                    icon="play"
                    disabled={playing}
                    onClick={() => stories.playFromFolder(entry)}
                  >
                    {text.playFromFolder}
                  </Button>
                )}
                <Button icon="download" onClick={() => stories.install(entry)}>
                  {fromCatalog ? text.download : text.install}
                </Button>
                <Button
                  variant="accent"
                  icon="play"
                  disabled={playing}
                  onClick={() => stories.install(entry, { thenPlay: true })}
                >
                  {fromCatalog ? text.downloadAndPlay : text.installAndPlay}
                </Button>
              </>
            )}
          </>
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
            <ul className="fl-detail__badges" aria-label={t.stories.detail.facts.state}>
              <li>
                <Badge tone={tone} icon={STATE_APPEARANCE[state].icon}>
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
                {progress.text && <p>{progress.text}</p>}
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
                <p>{failed.error.message}</p>
              </Banner>
            )}
            {playing && !playingThis && !task && (
              <p className="fl-detail__hint">{t.stories.playerBusy}</p>
            )}
          </div>
        </div>
      </Dialog>
      <Dialog
        open={confirmRemove}
        onClose={() => setConfirmRemove(false)}
        role="alertdialog"
        title={t.stories.removeDialog.title(entry.title)}
        description={t.stories.removeDialog.description}
        initialFocus={cancelRef}
        footer={
          <>
            <Button ref={cancelRef} onClick={() => setConfirmRemove(false)}>
              {t.common.cancel}
            </Button>
            <Button variant="danger" onClick={() => void confirmedRemove()}>
              {t.stories.removeDialog.confirm}
            </Button>
          </>
        }
      />
    </>
  );
}

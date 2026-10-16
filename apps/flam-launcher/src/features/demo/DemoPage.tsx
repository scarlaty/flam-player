import { useEffect, useRef, useState } from "react";
import {
  AppShell,
  Badge,
  Button,
  Dialog,
  EmptyState,
  IconButton,
  ProgressBar,
  SidebarSourceList,
  StoryCard,
  TaskPanel,
  ToastProvider,
  useToast,
  type SidebarSource,
  type Task,
  type TaskState,
} from "../../components";
import { t } from "../../i18n";
import { demoFr } from "../../i18n/demo.fr";
import { applyTheme, THEME_PREFERENCES, type ThemePreference } from "../../theme/theme";
import { demoSources, demoStories, demoTasks } from "./demoData";
import "./DemoPage.css";

/**
 * Vitrine des composants, chargée uniquement en développement (`#/demo`, voir App.tsx).
 * Les données sont fictives.
 */
export function DemoPage() {
  return (
    <ToastProvider>
      <DemoContent />
    </ToastProvider>
  );
}

function ThemeSwitcher() {
  const [theme, setTheme] = useState<ThemePreference>("system");

  useEffect(() => {
    applyTheme(theme);
  }, [theme]);

  return (
    <fieldset className="demo-theme">
      <legend className="fl-visually-hidden">{t.theme.label}</legend>
      {THEME_PREFERENCES.map((value) => (
        <label key={value} className="demo-theme__option">
          <input
            type="radio"
            name="demo-theme"
            value={value}
            checked={theme === value}
            onChange={() => setTheme(value)}
          />
          <span>{t.theme[value]}</span>
        </label>
      ))}
    </fieldset>
  );
}

function DemoContent() {
  const toast = useToast();
  const [sources, setSources] = useState<SidebarSource[]>(demoSources);
  const [sourceId, setSourceId] = useState("library");
  const [storyId, setStoryId] = useState("lucioles");
  const [dialogOpen, setDialogOpen] = useState(false);
  const [toDelete, setToDelete] = useState<SidebarSource | null>(null);
  const [tasks, setTasks] = useState<Task[]>(demoTasks);
  const cancelDeleteRef = useRef<HTMLButtonElement>(null);

  const setTaskState = (id: string, state: TaskState) =>
    setTasks((list) => list.map((task) => (task.id === id ? { ...task, state } : task)));
  const removeTask = (id: string) =>
    setTasks((list) => list.filter((task) => task.id !== id));
  const moveUp = (id: string) =>
    setSources((list) => {
      const index = list.findIndex((source) => source.id === id);
      if (index <= 0) return list;
      const next = [...list];
      [next[index - 1], next[index]] = [next[index]!, next[index - 1]!];
      return next;
    });

  return (
    <AppShell
      title={demoFr.title}
      sidebar={
        <SidebarSourceList
          sources={sources}
          selectedId={sourceId}
          onSelect={setSourceId}
          onAdd={() => setDialogOpen(true)}
          renderActions={(source) =>
            source.kind === "library" ? null : (
              <>
                <IconButton
                  icon="arrowUp"
                  size="sm"
                  label={demoFr.moveUp(source.label)}
                  onClick={() => moveUp(source.id)}
                />
                <IconButton icon="pencil" size="sm" label={demoFr.rename(source.label)} />
                <IconButton
                  icon="trash"
                  size="sm"
                  label={demoFr.deleteSource(source.label)}
                  onClick={() => setToDelete(source)}
                />
              </>
            )
          }
        />
      }
      headerActions={
        <>
          <ThemeSwitcher />
          <IconButton icon="refresh" label={demoFr.refresh} />
          <IconButton icon="sliders" label={demoFr.settings} />
        </>
      }
      aside={
        <TaskPanel
          tasks={tasks}
          onCancel={(id) => setTaskState(id, "cancelled")}
          onRetry={(id) => setTaskState(id, "queued")}
          onDismiss={removeTask}
        />
      }
    >
      <p className="demo-notice">{demoFr.notice}</p>

      <section className="demo-section" aria-labelledby="demo-stories">
        <h2 id="demo-stories">{demoFr.stories}</h2>
        <ul className="demo-grid">
          {demoStories.map((story) => (
            <li key={story.id}>
              <StoryCard
                title={story.title}
                coverUrl={story.coverUrl}
                ageMin={story.ageMin}
                ageMax={story.ageMax}
                badges={story.badges}
                progress={story.progress}
                progressText={
                  story.progress === "indeterminate" ? demoFr.converting : undefined
                }
                selected={story.id === storyId}
                onOpen={() => setStoryId(story.id)}
                actions={
                  <IconButton
                    icon="play"
                    variant="accent"
                    size="sm"
                    label={`${demoFr.play} : ${story.title}`}
                  />
                }
              />
            </li>
          ))}
        </ul>
      </section>

      <section className="demo-section" aria-labelledby="demo-buttons">
        <h2 id="demo-buttons">{demoFr.buttons}</h2>
        <div className="demo-row">
          <Button variant="primary" icon="download">
            {demoFr.install}
          </Button>
          <Button variant="accent" icon="play">
            {demoFr.play}
          </Button>
          <Button variant="ghost" icon="refresh">
            {demoFr.refresh}
          </Button>
          <Button variant="danger" icon="trash">
            {demoFr.delete}
          </Button>
          <Button variant="primary" disabled>
            {demoFr.install}
          </Button>
          <Button size="sm" icon="plus">
            {t.sources.add}
          </Button>
          <IconButton icon="sliders" variant="secondary" label={demoFr.settings} />
          <IconButton icon="play" variant="accent" label={demoFr.play} />
        </div>
      </section>

      <section className="demo-section" aria-labelledby="demo-badges">
        <h2 id="demo-badges">{demoFr.badges}</h2>
        <div className="demo-row">
          <Badge>{t.status.available}</Badge>
          <Badge tone="info">{t.status.compatible}</Badge>
          <Badge tone="success" icon="check">
            {t.status.installed}
          </Badge>
          <Badge tone="secondary">{t.status.new}</Badge>
          <Badge tone="warning">{t.status.partial}</Badge>
          <Badge tone="danger" icon="alert">
            {t.status.incompatible}
          </Badge>
        </div>
      </section>

      <section className="demo-section" aria-labelledby="demo-progress">
        <h2 id="demo-progress">{demoFr.progress}</h2>
        <div className="demo-stack">
          <ProgressBar value={35} label={demoFr.progressDownload} showValue />
          <ProgressBar value={100} tone="success" label={t.tasks.state.done} showValue />
          <ProgressBar label={demoFr.progressWaiting} />
        </div>
      </section>

      <section className="demo-section" aria-labelledby="demo-empty">
        <h2 id="demo-empty">{demoFr.empty}</h2>
        <div className="demo-panel">
          <EmptyState
            icon="folder"
            headingLevel={3}
            title={demoFr.emptyTitle}
            description={demoFr.emptyBody}
            action={
              <Button variant="primary" icon="plus" onClick={() => setDialogOpen(true)}>
                {t.sources.add}
              </Button>
            }
          />
        </div>
      </section>

      <section className="demo-section" aria-labelledby="demo-dialogs">
        <h2 id="demo-dialogs">{demoFr.dialogs}</h2>
        <div className="demo-row">
          <Button onClick={() => setDialogOpen(true)}>{demoFr.openDialog}</Button>
          <Button
            onClick={() => toast.show({ message: demoFr.toastDone, tone: "success" })}
          >
            {demoFr.showToast}
          </Button>
          <Button
            onClick={() => toast.show({ message: demoFr.toastError, tone: "danger" })}
          >
            {demoFr.showErrorToast}
          </Button>
          <Button
            variant="danger"
            icon="trash"
            onClick={() =>
              setToDelete(sources.find((source) => source.kind !== "library") ?? null)
            }
          >
            {demoFr.openAlertDialog}
          </Button>
        </div>
      </section>

      <Dialog
        open={dialogOpen}
        onClose={() => setDialogOpen(false)}
        title={demoFr.dialogTitle}
        description={demoFr.dialogDescription}
        footer={
          <>
            <Button variant="ghost" onClick={() => setDialogOpen(false)}>
              {t.common.cancel}
            </Button>
            <Button
              variant="primary"
              icon="download"
              onClick={() => {
                setDialogOpen(false);
                toast.show({ message: demoFr.toastDone, tone: "success" });
              }}
            >
              {demoFr.install}
            </Button>
          </>
        }
      >
        <p>{demoFr.dialogBody}</p>
      </Dialog>

      <Dialog
        role="alertdialog"
        open={toDelete !== null}
        onClose={() => setToDelete(null)}
        title={demoFr.alertTitle}
        description={demoFr.alertDescription}
        initialFocus={cancelDeleteRef}
        footer={
          <>
            <Button
              ref={cancelDeleteRef}
              variant="ghost"
              onClick={() => setToDelete(null)}
            >
              {t.common.cancel}
            </Button>
            <Button
              variant="danger"
              icon="trash"
              onClick={() => {
                const target = toDelete;
                setToDelete(null);
                if (target) {
                  setSources((list) => list.filter((source) => source.id !== target.id));
                }
              }}
            >
              {demoFr.delete}
            </Button>
          </>
        }
      />
    </AppShell>
  );
}

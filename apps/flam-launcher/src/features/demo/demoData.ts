import type { SidebarSource, StoryBadge, Task } from "../../components";
import { t } from "../../i18n";

/* Données fictives de la page de démonstration (titres inventés). */

export const demoSources: SidebarSource[] = [
  { id: "library", label: "Mes histoires", kind: "library", count: 12 },
  {
    id: "store",
    label: "Catalogue communautaire des histoires",
    kind: "catalog",
    count: 248,
  },
  { id: "mirror", label: "Catalogue de l'école", kind: "catalog", status: "offline" },
  { id: "downloads", label: "Téléchargements", kind: "folder", count: 3 },
  { id: "usb", label: "Clé USB", kind: "folder", status: "error" },
];

/** Vignette d'exemple en data: URI (autorisé par la CSP `img-src`). */
const demoCover =
  "data:image/svg+xml," +
  encodeURIComponent(
    '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">' +
      '<rect width="100" height="100" fill="#26395c"/>' +
      '<circle cx="70" cy="30" r="12" fill="#fff3c4"/>' +
      '<path d="M0 80 Q25 60 50 75 T100 70 V100 H0z" fill="#3d6b4f"/>' +
      '<path d="M0 90 Q30 78 60 88 T100 85 V100 H0z" fill="#2d523b"/>' +
      "</svg>",
  );

/** Image illisible : la carte doit se replier sur l'illustration générique. */
const brokenCover = "data:image/png;base64,AAAA";

const installed: StoryBadge = {
  id: "installed",
  label: t.status.installed,
  tone: "success",
  icon: "check",
};
const available: StoryBadge = { id: "available", label: t.status.available };

export interface DemoStory {
  id: string;
  title: string;
  coverUrl?: string;
  ageMin: number;
  ageMax?: number;
  badges: StoryBadge[];
  progress?: number | "indeterminate";
}

export const demoStories: DemoStory[] = [
  {
    id: "lucioles",
    title: "La forêt des lucioles",
    coverUrl: demoCover,
    ageMin: 3,
    ageMax: 6,
    badges: [installed, { id: "compat", label: t.status.compatible, tone: "info" }],
  },
  {
    id: "nuage",
    title: "Pipo le petit nuage",
    ageMin: 2,
    ageMax: 4,
    badges: [available],
    progress: "indeterminate",
  },
  {
    id: "mila",
    title: "Le grand voyage de Mila",
    ageMin: 5,
    badges: [
      { id: "new", label: t.status.new, tone: "warning", icon: "sparkle" },
      { id: "compat", label: t.status.partial, tone: "warning" },
    ],
    progress: 62,
  },
  {
    id: "ours",
    title: "Bonne nuit, petit ours",
    coverUrl: brokenCover,
    ageMin: 2,
    ageMax: 5,
    badges: [installed],
  },
  {
    id: "phare",
    title: "Les secrets du phare",
    ageMin: 6,
    ageMax: 8,
    badges: [
      { id: "compat", label: t.status.incompatible, tone: "danger", icon: "alert" },
    ],
  },
  {
    id: "toc",
    title: "Toc toc, qui est là ?",
    ageMin: 3,
    badges: [available],
  },
];

export const demoTasks: Task[] = [
  {
    id: "t1",
    title: "Le grand voyage de Mila",
    kind: "download",
    state: "running",
    progress: 62,
    detail: "18 Mo sur 29 Mo",
  },
  {
    id: "t2",
    title: "Pipo le petit nuage",
    kind: "convert",
    state: "running",
    phase: "Conversion des sons",
  },
  { id: "t3", title: "Toc toc, qui est là ?", kind: "install", state: "queued" },
  { id: "t4", title: "Bonne nuit, petit ours", kind: "install", state: "done" },
  {
    id: "t5",
    title: "Les secrets du phare",
    kind: "convert",
    state: "error",
    phase: "Lecture de l'archive",
    detail: "Format d'archive non reconnu.",
  },
  { id: "t6", title: "La forêt des lucioles", kind: "download", state: "cancelled" },
];

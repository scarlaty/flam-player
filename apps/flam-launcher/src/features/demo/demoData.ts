import type { SidebarSource, StoryBadge, Task } from "../../components";
import { t } from "../../i18n";

/* Données fictives de la page de démonstration (titres inventés). */

export const demoSources: SidebarSource[] = [
  { id: "library", label: "Mes histoires", kind: "library", count: 12 },
  { id: "store", label: "Catalogue communautaire", kind: "catalog", count: 248 },
  { id: "mirror", label: "Catalogue hors ligne", kind: "catalog", unavailable: true },
  { id: "downloads", label: "Téléchargements", kind: "folder", count: 3 },
  { id: "usb", label: "Clé USB", kind: "folder", count: 0 },
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

export interface DemoStory {
  id: string;
  title: string;
  coverUrl?: string;
  ageMin: number;
  ageMax?: number;
  badges: StoryBadge[];
}

export const demoStories: DemoStory[] = [
  {
    id: "lucioles",
    title: "La forêt des lucioles",
    coverUrl: demoCover,
    ageMin: 3,
    ageMax: 6,
    badges: [
      { label: t.status.installed, tone: "success", icon: "check" },
      { label: t.status.compatible, tone: "info" },
    ],
  },
  {
    id: "nuage",
    title: "Pipo le petit nuage",
    ageMin: 2,
    ageMax: 4,
    badges: [{ label: t.status.available }],
  },
  {
    id: "mila",
    title: "Le grand voyage de Mila",
    ageMin: 5,
    badges: [
      { label: t.status.new, tone: "warning", icon: "sparkle" },
      { label: t.status.partial, tone: "warning" },
    ],
  },
  {
    id: "ours",
    title: "Bonne nuit, petit ours",
    ageMin: 2,
    ageMax: 5,
    badges: [{ label: t.status.installed, tone: "success", icon: "check" }],
  },
  {
    id: "phare",
    title: "Les secrets du phare",
    ageMin: 6,
    ageMax: 8,
    badges: [{ label: t.status.incompatible, tone: "danger", icon: "alert" }],
  },
  {
    id: "toc",
    title: "Toc toc, qui est là ?",
    ageMin: 3,
    badges: [{ label: t.status.available }],
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
  { id: "t2", title: "Pipo le petit nuage", kind: "convert", state: "running" },
  { id: "t3", title: "Toc toc, qui est là ?", kind: "install", state: "queued" },
  { id: "t4", title: "Bonne nuit, petit ours", kind: "install", state: "done" },
  {
    id: "t5",
    title: "Les secrets du phare",
    kind: "convert",
    state: "error",
    detail: "Format d'archive non reconnu.",
  },
];

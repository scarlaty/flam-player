#!/usr/bin/env node
/**
 * Captures d'écran de flam-launcher pour le README racine (`docs/assets/`).
 *
 * Lance le front Vite (ou réutilise un serveur déjà démarré avec `--url`), remplace le backend
 * Tauri par un `window.__TAURI_INTERNALS__` simulé (même principe que `@tauri-apps/api/mocks`),
 * servi par des données de démonstration fictives (titres de `src/features/demo/demoData.ts`
 * complétés ci-dessous), puis photographie les écrans principaux dans Edge (Playwright, canal
 * `msedge` : aucun navigateur à télécharger), en thème clair et en thème sombre. Un GIF du
 * parcours (catalogue → fiche → installation → Ma Flam) est assemblé avec ffmpeg s'il est dans
 * le PATH ; sinon seules les images fixes sont produites.
 *
 * Usage (depuis apps/flam-launcher) :
 *   npm run screenshots
 *   npm run screenshots -- --url http://localhost:1420   (serveur Vite déjà lancé)
 *   npm run screenshots -- --port 14388 --out ../../docs/assets --no-gif
 *
 * Hors CI : le résultat dépend des polices et du rendu d'Edge du poste.
 */
import { spawnSync } from "node:child_process";
import { mkdir, mkdtemp, readFile, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";
import { chromium } from "playwright-core";

const APP_DIR = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const REPO_ROOT = path.resolve(APP_DIR, "../..");

const { values: options } = parseArgs({
  options: {
    url: { type: "string" },
    port: { type: "string", default: "14388" },
    out: { type: "string", default: path.join(REPO_ROOT, "docs", "assets") },
    "no-gif": { type: "boolean", default: false },
  },
});

const VIEWPORT = { width: 1440, height: 900 };
const OUT_DIR = path.resolve(options.out);

// ---------------------------------------------------------------------------------------------
// Données de démonstration (fictives : titres inventés, aucune donnée personnelle)
// ---------------------------------------------------------------------------------------------

/** Couleurs des vignettes : fond haut, fond bas, décor, astre. */
const PALETTES = [
  ["#26395c", "#4b5d8f", "#3d6b4f", "#fff3c4"],
  ["#f6b38e", "#f7d9a8", "#e0794f", "#fffaf0"],
  ["#7cc4e8", "#c9ecf7", "#2f7fb5", "#ffd65a"],
  ["#3b2a5c", "#7a4f9a", "#c86fb3", "#ffe8a3"],
  ["#9ed4a2", "#e3f3d4", "#4f9a5a", "#ffcf6e"],
  ["#1f4e5f", "#3f8a8c", "#f2c14e", "#f7f3e3"],
];

/** Décors des vignettes (SVG 100×100). */
const MOTIFS = [
  // Collines sous la lune
  (p) =>
    `<circle cx="70" cy="30" r="12" fill="${p[3]}"/>` +
    `<path d="M0 78 Q25 58 50 72 T100 66 V100 H0z" fill="${p[2]}"/>` +
    `<path d="M0 90 Q30 76 60 86 T100 82 V100 H0z" fill="#00000033"/>`,
  // Soleil sur la mer
  (p) =>
    `<circle cx="50" cy="58" r="20" fill="${p[3]}"/>` +
    `<path d="M0 62 Q12 56 25 62 T50 62 T75 62 T100 62 V100 H0z" fill="${p[2]}"/>` +
    `<path d="M0 76 Q12 70 25 76 T50 76 T75 76 T100 76 V100 H0z" fill="#ffffff55"/>`,
  // Montagnes enneigées
  (p) =>
    `<circle cx="26" cy="26" r="9" fill="${p[3]}"/>` +
    `<path d="M-5 100 L35 40 L60 76 L75 56 L105 100z" fill="${p[2]}"/>` +
    `<path d="M35 40 L27 52 L35 49 L42 54z" fill="#fff"/>`,
  // Nuit étoilée
  (p) =>
    [
      [18, 20, 1.6],
      [40, 12, 1.2],
      [64, 24, 1.8],
      [84, 14, 1.2],
      [28, 40, 1],
      [74, 42, 1.4],
    ]
      .map(([x, y, r]) => `<circle cx="${x}" cy="${y}" r="${r}" fill="${p[3]}"/>`)
      .join("") +
    `<path d="M66 28 a14 14 0 1 0 12 22 a11 11 0 1 1 -12 -22z" fill="${p[3]}"/>` +
    `<path d="M0 84 Q50 70 100 84 V100 H0z" fill="${p[2]}"/>`,
  // Arbre
  (p) =>
    `<circle cx="78" cy="24" r="10" fill="${p[3]}"/>` +
    `<rect x="45" y="56" width="10" height="30" rx="3" fill="#6b4a2f"/>` +
    `<circle cx="50" cy="48" r="22" fill="${p[2]}"/>` +
    `<path d="M0 86 Q50 78 100 86 V100 H0z" fill="#00000026"/>`,
  // Montgolfière
  (p) =>
    `<ellipse cx="50" cy="38" rx="18" ry="21" fill="${p[2]}"/>` +
    `<path d="M36 50 L46 66 H54 L64 50z" fill="${p[2]}" opacity=".7"/>` +
    `<rect x="45" y="66" width="10" height="8" rx="2" fill="#6b4a2f"/>` +
    `<circle cx="20" cy="76" r="8" fill="#ffffffaa"/><circle cx="30" cy="78" r="10" fill="#ffffffaa"/>` +
    `<circle cx="80" cy="22" r="6" fill="${p[3]}"/>`,
];

function coverDataUrl(index) {
  // Couples (couleurs, décor) tous différents sur les 16 premières vignettes.
  const palette = PALETTES[index % PALETTES.length];
  const motif =
    MOTIFS[(index * 5 + 2 * Math.floor(index / PALETTES.length) + 1) % MOTIFS.length];
  const svg =
    '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">' +
    `<defs><linearGradient id="g" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="${palette[0]}"/><stop offset="1" stop-color="${palette[1]}"/></linearGradient></defs>` +
    `<rect width="100" height="100" fill="url(#g)"/>${motif(palette)}</svg>`;
  return `data:image/svg+xml,${encodeURIComponent(svg)}`;
}

/**
 * Détails des histoires de `demoData.ts` (par identifiant) puis histoires supplémentaires.
 * Les titres de `demoData.ts` sont lus dans le navigateur, via Vite, au premier appel.
 */
const DEMO_DETAILS = {
  lucioles: {
    description:
      "Une nuit d'été, Lila suit une luciole jusqu'au cœur de la forêt. Une histoire douce pour s'endormir.",
    category: "Douce nuit",
  },
  nuage: {
    description:
      "Pipo voudrait pleuvoir sur le jardin de son amie la tortue. Mais comment faire ?",
    category: "Animaux",
  },
  mila: {
    description:
      "Mila traverse mers et montagnes pour retrouver sa grand-mère. À chaque étape, l'enfant choisit la route.",
    category: "Aventure",
  },
  ours: {
    description:
      "Petit ours n'arrive pas à dormir. Ses amis de la forêt viennent l'aider.",
    category: "Douce nuit",
  },
  phare: {
    description:
      "Une énigme à résoudre en haut d'un vieux phare, entre mouettes et tempête.",
    category: "Énigmes",
  },
  toc: {
    description:
      "Qui frappe à la porte ? Une histoire à rebondissements pour les plus petits.",
    category: "Rire",
  },
};

const EXTRA_STORIES = [
  {
    title: "Le potager de Lou",
    age: 3,
    category: "Découverte",
    description:
      "Lou plante, arrose et attend. Un conte pour découvrir les saisons au jardin.",
  },
  {
    title: "La chorale des grenouilles",
    age: 4,
    category: "Musique",
    description:
      "Au bord de l'étang, chaque grenouille cherche sa note. L'enfant compose le chant.",
  },
  {
    title: "Capitaine Noisette",
    age: 5,
    category: "Aventure",
    description:
      "Un écureuil pirate part à la recherche du gland d'or. Trois fins possibles.",
  },
  {
    title: "Le voyage de la petite tortue",
    age: 3,
    category: "Animaux",
    description:
      "Une tortue rejoint la mer en prenant son temps. Une histoire sur la patience.",
  },
  {
    title: "Zélie et la graine magique",
    age: 4,
    category: "Merveilleux",
    description:
      "Une graine tombée du ciel pousse jusqu'aux nuages. Zélie grimpe pour voir.",
  },
  {
    title: "La montagne qui chantait",
    age: 6,
    category: "Aventure",
    description:
      "Au sommet, un écho répond aux questions. Mais il faut poser les bonnes.",
  },
];

/** Histoires officielles fictives présentes sur la Flam (titres inventés). */
const DEVICE_ONLY = [
  "Le château des nuages",
  "Promenade au bord de l'eau",
  "Contes du soir au coin du feu",
];

const CONFIG = {
  version: JSON.parse(await readFile(path.join(APP_DIR, "package.json"), "utf8")).version,
  covers: Array.from({ length: 16 }, (_, i) => coverDataUrl(i)),
  details: DEMO_DETAILS,
  extras: EXTRA_STORIES,
  deviceOnly: DEVICE_ONLY,
};

// ---------------------------------------------------------------------------------------------
// Backend simulé (exécuté dans la page avant l'application ; aucun accès à Node ici)
// ---------------------------------------------------------------------------------------------

function installMockBackend({ config, scenario }) {
  const g = globalThis;
  const callbacks = new Map();
  const listeners = new Map();
  const unknown = new Set();
  let nextCallback = 1;

  function transformCallback(callback, once = false) {
    const id = nextCallback++;
    callbacks.set(id, (data) => {
      if (once) callbacks.delete(id);
      return callback?.(data);
    });
    return id;
  }

  function emit(event, payload) {
    for (const id of listeners.get(event) ?? [])
      callbacks.get(id)?.({ event, id, payload });
  }

  const CATALOG_ID = "catalogue";
  const FOLDER_ID = "cle-usb";
  const NOW = Math.floor(Date.UTC(2026, 9, 20, 17, 30) / 1000);
  /** UUID fictif et stable, différent dès les huit premiers caractères. */
  const uuid = (n) => {
    const hex = ((n * 2654435761) >>> 0).toString(16).padStart(8, "0");
    return `${hex}-5d1e-4c7b-9a0f-${String(n).padStart(12, "0")}`;
  };

  let state = null;

  async function data() {
    if (state) return state;
    // Titres et âges des histoires de la page de démonstration (servie par Vite).
    const { demoStories } = await import("/src/features/demo/demoData.ts");
    const base = demoStories.map((story) => ({
      title: story.title,
      age: story.ageMin,
      ...(config.details[story.id] ?? {
        description: "Une histoire à écouter et à jouer.",
        category: "Aventure",
      }),
    }));
    const catalog = [...base, ...config.extras].map((story, i) => ({
      source_id: CATALOG_ID,
      entry_id: `${CATALOG_ID}:${uuid(i + 1)}`,
      uuid: uuid(i + 1),
      version: 2,
      title: story.title,
      description: story.description,
      age: story.age,
      category: story.category,
      author: "Atelier des conteurs",
      voice: "Camille",
      license: "CC BY-NC-SA 4.0",
      thumb_url: `cover-${i}`,
      download_url: `https://catalogue.example.org/histoires/${i + 1}.zip`,
      size: 18_000_000 + ((i * 7_300_000) % 41_000_000),
      updated_at: `2026-0${(i % 9) + 1}-1${i % 9}T09:00:00Z`,
      download_count: 4200 - i * 290,
      kind: "telmi_zip",
    }));
    const cover = (entry) => config.covers[catalog.indexOf(entry) % config.covers.length];
    const libraryEntry = (entry, days) => ({
      uuid: entry.uuid,
      version: entry.version,
      title: entry.title,
      file: `${entry.uuid}-${entry.version}.plain.pk`,
      size: Math.round((entry.size ?? 20_000_000) * 0.8),
      installed_at: NOW - days * 86400,
      converter: "telmi2flam 0.5.0",
      source: entry,
      thumb: `${entry.uuid}.png`,
      source_stamp: null,
    });
    const installedIdx = scenario.installed ?? [0, 3, 6, 9, 10];
    const library = installedIdx.map((i, n) => libraryEntry(catalog[i], n * 3 + 1));
    const folder = [7, 8, 11].map((i, n) => ({
      ...catalog[i],
      source_id: FOLDER_ID,
      entry_id: `${FOLDER_ID}:${n}`,
      thumb_url: cover(catalog[i]),
      download_url: `E:\\Histoires\\${catalog[i].title}.zip`,
      download_count: null,
    }));
    const deviceStories = [
      ...library.slice(0, 2).map((item) => ({
        uuid: item.uuid,
        short_id: item.uuid.slice(0, 8),
        placement: "visible",
        title: item.title,
        in_library: true,
        missing: false,
        library_uuids: [item.uuid],
      })),
      ...config.deviceOnly.map((title, i) => ({
        uuid: uuid(100 + i),
        short_id: uuid(100 + i).slice(0, 8),
        placement: "visible",
        title: null,
        in_library: false,
        missing: false,
        library_uuids: [],
        officialTitle: title,
        cover: config.covers[(i + 12) % config.covers.length],
      })),
    ];
    state = { catalog, cover, library, libraryEntry, folder, deviceStories, tasks: [] };
    for (const task of scenario.tasks ?? []) {
      state.tasks.push({
        id: `t${task.index}`,
        entry: catalog[task.index],
        state: task.state,
        percent: task.percent ?? null,
        error: null,
        created_at: NOW,
      });
    }
    return state;
  }

  const sources = [
    { type: "library", id: "library", name: "Mes histoires" },
    {
      type: "catalog",
      id: CATALOG_ID,
      name: "Catalogue communautaire",
      url: "https://catalogue.example.org/histoires.json",
    },
    { type: "folder", id: FOLDER_ID, name: "Clé USB", path: "E:\\Histoires" },
  ];

  const settings = {
    schema_version: 1,
    player_path: null,
    cache_dir: null,
    default_catalog_url: "https://catalogue.example.org/histoires.json",
    theme: "system",
    max_downloads: 2,
    thumb_fit: "flou",
    carrier_consent: { text_version: 1, accepted_at: "2026-10-01T18:00:00Z" },
  };
  const installDir = "C:\\Users\\demo\\AppData\\Local\\Flam Launcher";
  const settingsView = () => ({
    settings,
    resolved: {
      player_path: `${installDir}\\flam-player.exe`,
      cache_dir: "C:\\Users\\demo\\AppData\\Local\\flam-launcher\\cache",
    },
    issues: [],
    recovery: null,
  });

  const firmware = {
    version: "1.15.14",
    family: "v1",
    version_form: "main",
    mdf_size: 126,
    header: "01 00",
    layout_standard: true,
    usb_ids: null,
    coherence: { kind: "not_applicable" },
    write_block: null,
  };
  const device = {
    root: "F:\\",
    volume_label: "FLAM",
    file_system: "FAT32",
    space: { free: 5_420_000_000, total: 7_730_000_000 },
    warnings: [],
    mdf_problem: null,
    backup_failed: false,
    firmware: firmware.version,
    firmware_status: firmware,
  };

  /** Déroulé simulé d'une installation : téléchargement, conversion, installation. */
  function runInstall(task) {
    const s = state;
    const total = task.entry.size ?? 20_000_000;
    const steps = [];
    for (let p = 0; p <= 100; p += 20)
      steps.push({
        state: "downloading",
        percent: p,
        received: Math.round((total * p) / 100),
        total,
      });
    for (const [phase, percent] of [
      ["images", 15],
      ["audio", 45],
      ["audio", 75],
      ["scripts", 90],
      ["zip", 100],
    ])
      steps.push({ state: "converting", percent, phase });
    steps.push({ state: "installing", percent: null });
    let i = 0;
    const tick = () => {
      if (i < steps.length) {
        const step = steps[i++];
        task.state = step.state;
        task.percent = step.percent;
        emit("install://progress", {
          task_id: task.id,
          state: step.state,
          percent: step.percent,
          received: step.received ?? null,
          total: step.total ?? null,
          phase: step.phase ?? null,
        });
        setTimeout(tick, scenario.stepMs ?? 250);
        return;
      }
      task.state = "done";
      const entry = s.libraryEntry(task.entry, 0);
      s.library.push(entry);
      s.library.sort((a, b) => a.title.localeCompare(b.title, "fr"));
      emit("install://done", { task_id: task.id, entry, warnings: 0 });
    };
    setTimeout(tick, scenario.stepMs ?? 250);
  }

  const handlers = {
    app_info: () => ({ name: "Flam Launcher", version: config.version }),
    settings_get: settingsView,
    settings_set: (args) => {
      Object.assign(settings, args.settings);
      return settingsView();
    },
    // Voix des titres lus (#462) : voix fictives, aucun son.
    tts_voices: () => ({
      voices: [
        { id: "voix-hortense", name: "Microsoft Hortense", language: "fr-FR" },
        { id: "voix-paul", name: "Microsoft Paul", language: "fr-FR" },
      ],
      default_id: "voix-hortense",
    }),
    tts_preview: () => ({ played: false, duration_ms: 0 }),
    tts_preview_stop: () => null,
    sources_list: () => ({ sources, recovery: null }),
    config_ack_recovery: () => null,
    podcast_mp3_cache_size: () => ({ bytes: 96_000_000 }),
    cache_usage: () => ({
      path: "C:\\Users\\demo\\AppData\\Local\\flam-launcher\\cache",
      bytes: 412_000_000,
      subdirs: ["catalogs", "downloads", "library", "thumbs"],
    }),
    catalog_entries: async (args) => catalogView(args.sourceId),
    catalog_refresh: async (args) => catalogView(args.sourceId),
    catalog_thumbnail: async (args) => {
      const s = await data();
      const index = Number(String(args.thumbUrl).replace("cover-", ""));
      return config.covers[index % config.covers.length] ?? s.cover(s.catalog[0]);
    },
    folder_scan: async () => ({
      entries: (await data()).folder,
      warnings: [],
    }),
    folder_thumbnail: async (args) =>
      (await data()).folder.find((e) => e.entry_id === args.entryId)?.thumb_url,
    folder_watch: () => null,
    library_list: async () => (await data()).library,
    library_source_status: async () =>
      (await data()).library.map((e) => ({ uuid: e.uuid, state: "unknown" })),
    library_thumbnail: async (args) => {
      const s = await data();
      const entry = s.catalog.find((e) => e.uuid === args.uuid);
      if (!entry) throw { kind: "not_found", message: "Vignette absente" };
      return s.cover(entry);
    },
    compat_statuses: async () => {
      const s = await data();
      const level = (i, lvl, reason) => ({
        uuid: s.catalog[i].uuid,
        version: "2",
        level: lvl,
        reason,
        code: null,
        origin: "campaign",
        stale: false,
      });
      return {
        entries: [
          level(0, "compatible", null),
          level(2, "warnings", "Quelques images redimensionnées."),
          level(3, "compatible", null),
          level(4, "incompatible", "Archive incomplète : sons manquants."),
          level(6, "compatible", null),
          level(8, "compatible", null),
          level(9, "compatible", null),
        ],
        issue: null,
      };
    },
    install_list: async () => {
      const s = await data();
      // Tâches du scénario : leur avancement détaillé arrive par événement, comme en vrai.
      setTimeout(() => {
        for (const task of scenario.tasks ?? []) {
          emit("install://progress", {
            task_id: `t${task.index}`,
            state: task.state,
            percent: task.percent ?? null,
            received: task.received ?? null,
            total: task.total ?? null,
            phase: task.phase ?? null,
          });
        }
      }, 200);
      return s.tasks;
    },
    install_start: async (args) => {
      const s = await data();
      const entry = s.catalog.find((e) => e.entry_id === args.entryId);
      const task = {
        id: `t${s.tasks.length + 100}`,
        entry,
        state: "queued",
        percent: null,
        error: null,
        created_at: NOW,
      };
      s.tasks.push(task);
      runInstall(task);
      return { ...task };
    },
    install_cancel: () => null,
    player_status: () => null,
    player_launch: () => {
      throw {
        kind: "player_unavailable",
        message: "Lecture désactivée en démonstration.",
      };
    },
    device_status: () => ({
      detected: scenario.device !== false,
      hint: null,
      device: scenario.device !== false ? device : null,
      device_count: scenario.device !== false ? 1 : 0,
    }),
    device_list_stories: async () => {
      if (scenario.device === false) return null;
      const s = await data();
      return {
        stories: s.deviceStories.map((story) => {
          const copy = { ...story };
          delete copy.officialTitle;
          delete copy.cover;
          return copy;
        }),
        visible_index: "valid",
        hidden_index: "valid",
        truncated: false,
        limit: 500,
        str_unreadable: false,
      };
    },
    device_story_titles: async (args) => {
      const s = await data();
      return {
        titles: args.uuids.map((id) => {
          const story = s.deviceStories.find((d) => d.uuid === id);
          return { uuid: id, title: story?.title ?? story?.officialTitle ?? null };
        }),
        interrupted: false,
        keys: true,
      };
    },
    device_story_thumbnail: async (args) => {
      const s = await data();
      const story = s.deviceStories.find((d) => d.uuid === args.uuid);
      const entry = s.catalog.find((e) => e.uuid === args.uuid);
      return { thumb: story?.cover ?? (entry ? s.cover(entry) : null), busy: false };
    },
    device_official_titles: () => ({ titles: [] }),
    device_official_thumbnail: () => null,
    device_story_details_cancel: () => null,
    device_carrier_state: () => ({
      warning_text: "Fonction expérimentale.",
      warning_version: 1,
      consent: settings.carrier_consent,
      firmware: firmware.version,
      carrier_needed: true,
      stored_keys: true,
      candidates: [],
    }),
    device_send_story: async (args) => {
      const s = await data();
      const item = s.library.find((e) => e.uuid === args.uuid);
      const total = 40;
      for (const phase of ["copy", "verify"]) {
        for (let done = 0; done <= total; done += 8) {
          await new Promise((resolve) => setTimeout(resolve, scenario.stepMs ?? 250));
          emit("device://transfer-progress", { uuid: args.uuid, phase, done, total });
        }
      }
      s.deviceStories.push({
        uuid: item.uuid,
        short_id: item.uuid.slice(0, 8),
        placement: "visible",
        title: item.title,
        in_library: true,
        missing: false,
        library_uuids: [item.uuid],
      });
      return {
        id: item.uuid,
        bytes: item.size,
        files: total,
        replaced: false,
        index_updated: true,
        cache_removed: true,
        carrier_readback: true,
        carrier_stored: true,
        recovery_failures: [],
      };
    },
    device_recover_transfers: () => ({
      incomplete_removed: 0,
      previous_restored: 0,
      updates_finished: 0,
      left_alone: 0,
    }),
    device_diagnostic: () => "",
    "plugin:event|listen": (args) => {
      if (!listeners.has(args.event)) listeners.set(args.event, new Set());
      listeners.get(args.event).add(args.handler);
      return args.handler;
    },
    "plugin:event|unlisten": (args) => {
      listeners.get(args.event)?.delete(args.eventId);
      return null;
    },
  };

  async function catalogView(sourceId) {
    const s = await data();
    return {
      source_id: sourceId,
      entries: s.catalog,
      warnings: [],
      status: { offline: false, updated_at: NOW, error: null },
    };
  }

  async function invoke(cmd, args = {}) {
    const handler = handlers[cmd];
    if (!handler) {
      unknown.add(cmd);
      return null;
    }
    return handler(args);
  }

  g.__TAURI_INTERNALS__ = {
    invoke,
    transformCallback,
    unregisterCallback: (id) => callbacks.delete(id),
    runCallback: (id, payload) => callbacks.get(id)?.(payload),
    callbacks,
    convertFileSrc: (file) => file,
    metadata: {
      currentWindow: { label: "main" },
      currentWebview: { windowLabel: "main", label: "main" },
    },
  };
  g.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
    unregisterListener: (event, id) => {
      listeners.get(event)?.delete(id);
      callbacks.delete(id);
    },
  };
  g.__flamDemo = { emit, unknown: () => [...unknown] };
}

// ---------------------------------------------------------------------------------------------
// Pilotage
// ---------------------------------------------------------------------------------------------

/** Écrans photographiés : nom de fichier, adresse, scénario et préparation éventuelle. */
const SHOTS = [
  {
    name: "catalogue",
    hash: "#/library/catalogue",
    ready: ".fl-library__item img",
    scenario: { device: false },
  },
  {
    name: "fiche",
    hash: "#/library/catalogue",
    ready: ".fl-library__item img",
    scenario: { device: false },
    prepare: async (page) => {
      await page
        .getByRole("button", { name: /Capitaine Noisette/ })
        .first()
        .click();
    },
  },
  {
    name: "installation",
    hash: "#/library/catalogue",
    ready: ".fl-library__item img",
    scenario: {
      device: false,
      tasks: [
        {
          index: 2,
          state: "downloading",
          percent: 62,
          received: 19_300_000,
          total: 31_100_000,
        },
        { index: 1, state: "converting", percent: 45, phase: "audio" },
        { index: 5, state: "queued" },
      ],
    },
  },
  { name: "mes-histoires", hash: "#/library/library", ready: ".fl-library__item img" },
  { name: "ma-flam", hash: "#/device", ready: "img" },
  { name: "sources", hash: "#/sources", ready: "main li" },
  { name: "parametres", hash: "#/settings", ready: "main select, main input" },
];

const THEMES = [
  { name: "clair", colorScheme: "light" },
  { name: "sombre", colorScheme: "dark" },
];

async function startServer() {
  if (options.url) return { url: options.url.replace(/\/$/, ""), close: async () => {} };
  const { createServer } = await import("vite");
  const server = await createServer({
    root: APP_DIR,
    configFile: path.join(APP_DIR, "vite.config.ts"),
    logLevel: "warn",
    server: { port: Number(options.port), strictPort: true, open: false },
  });
  await server.listen();
  return { url: `http://localhost:${options.port}`, close: () => server.close() };
}

async function openPage(browser, baseUrl, { colorScheme, scenario = {}, hash }) {
  const context = await browser.newContext({
    viewport: VIEWPORT,
    deviceScaleFactor: 1,
    colorScheme,
    locale: "fr-FR",
    reducedMotion: "reduce",
  });
  await context.addInitScript(installMockBackend, { config: CONFIG, scenario });
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto(`${baseUrl}/${hash}`);
  return { context, page, errors };
}

/** Attend la fin des chargements visibles (vignettes, « Chargement… »). */
async function settle(page, ready) {
  await page.waitForSelector(ready, { timeout: 15000 });
  await page.waitForFunction(
    () => !globalThis.document.body.innerText.includes("Chargement"),
    null,
    {
      timeout: 15000,
    },
  );
  await page.waitForTimeout(600);
}

/** Problèmes relevés pendant les captures : le script sort alors en échec. */
let problems = 0;

async function report(page, errors, label) {
  const unknown = await page.evaluate(() => globalThis.__flamDemo.unknown());
  if (unknown.length) {
    problems++;
    console.error(`  ${label} : commandes non simulées : ${unknown.join(", ")}`);
  }
  for (const error of errors) {
    problems++;
    console.error(`  ${label} : erreur de page : ${error}`);
  }
}

async function captureStills(browser, baseUrl) {
  const written = [];
  for (const shot of SHOTS) {
    for (const theme of THEMES) {
      const label = `${shot.name}-${theme.name}`;
      const { context, page, errors } = await openPage(browser, baseUrl, {
        colorScheme: theme.colorScheme,
        scenario: shot.scenario,
        hash: shot.hash,
      });
      await settle(page, shot.ready);
      if (shot.prepare) {
        await shot.prepare(page);
        await page.waitForTimeout(600);
      }
      const file = path.join(OUT_DIR, `${label}.png`);
      await page.screenshot({ path: file });
      await report(page, errors, label);
      await context.close();
      written.push(file);
    }
  }
  return written;
}

/** Parcours animé : liste d'images et durée d'affichage de chacune (en secondes). */
async function captureJourney(browser, baseUrl, framesDir) {
  const { context, page, errors } = await openPage(browser, baseUrl, {
    colorScheme: "light",
    scenario: { stepMs: 220 },
    hash: "#/library/catalogue",
  });
  const frames = [];
  const snap = async (seconds) => {
    const file = path.join(
      framesDir,
      `frame-${String(frames.length).padStart(3, "0")}.png`,
    );
    await page.screenshot({ path: file });
    frames.push({ file, seconds });
  };
  await settle(page, ".fl-library__item img");
  await snap(2);
  await page
    .getByRole("button", { name: /Capitaine Noisette/ })
    .first()
    .click();
  await page.waitForTimeout(500);
  await snap(2);
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "Télécharger", exact: true })
    .click();
  for (let i = 0; i < 14; i++) {
    await page.waitForTimeout(220);
    await snap(0.25);
  }
  await page.waitForTimeout(800);
  await snap(1.5);
  // Envoi sur la Flam (1.x : choix des clés du mode carrier, conservées sur ce PC).
  await page
    .getByRole("dialog")
    .getByRole("button", { name: /Envoyer sur la Flam/ })
    .first()
    .click();
  // Avertissement « fonction expérimentale » avant le premier envoi.
  const warning = page.getByRole("button", { name: /J'ai compris, envoyer/ });
  await warning.waitFor({ timeout: 5000 });
  await page.waitForTimeout(300);
  await snap(2.5);
  await warning.click();
  const carrier = page.getByRole("alertdialog", { name: /mode carrier/ });
  const carrierShown = await carrier
    .getByRole("radio", { name: /Clés conservées/ })
    .waitFor({ timeout: 5000 })
    .then(
      () => true,
      () => false,
    );
  if (carrierShown) {
    await carrier.getByRole("radio", { name: /Clés conservées/ }).check();
    await page.waitForTimeout(300);
    await snap(2.5);
    await carrier.getByRole("button", { name: /Vérifier et envoyer/ }).click();
  }
  for (let i = 0; i < 10; i++) {
    await page.waitForTimeout(440);
    await snap(0.4);
  }
  await page.waitForTimeout(800);
  await page.keyboard.press("Escape");
  await page.evaluate(() => {
    globalThis.location.hash = "#/device";
  });
  await settle(page, "img");
  // Liste des histoires de la Flam en haut de l'écran : la nouvelle y figure.
  await page
    .getByRole("heading", { name: /Histoires sur la Flam/ })
    .evaluate((node) => node.scrollIntoView({ block: "start" }));
  await page.waitForTimeout(400);
  await snap(3);
  await report(page, errors, "parcours");
  await context.close();
  return frames;
}

function findFfmpeg() {
  const candidate = process.env.FFMPEG ?? "ffmpeg";
  const probe = spawnSync(candidate, ["-version"], { stdio: "ignore" });
  return probe.status === 0 ? candidate : null;
}

async function buildGif(ffmpeg, frames, framesDir, output) {
  const list = frames
    .map((f) => `file '${f.file.replaceAll("\\", "/")}'\nduration ${f.seconds}`)
    .join("\n");
  // Le démultiplexeur concat ignore la durée de la dernière image sans cette répétition.
  const listFile = path.join(framesDir, "frames.txt");
  await writeFile(
    listFile,
    `${list}\nfile '${frames.at(-1).file.replaceAll("\\", "/")}'\n`,
  );
  const filter =
    "scale=960:-1:flags=lanczos,split[a][b];[a]palettegen=max_colors=128:stats_mode=diff[p];" +
    "[b][p]paletteuse=dither=bayer:bayer_scale=4:diff_mode=rectangle";
  const run = spawnSync(
    ffmpeg,
    [
      "-y",
      "-loglevel",
      "error",
      "-f",
      "concat",
      "-safe",
      "0",
      "-i",
      listFile,
      "-vf",
      filter,
      "-loop",
      "0",
      output,
    ],
    { stdio: "inherit" },
  );
  if (run.status !== 0) throw new Error(`ffmpeg a échoué (code ${run.status})`);
}

async function main() {
  await mkdir(OUT_DIR, { recursive: true });
  const server = await startServer();
  let browser;
  try {
    browser = await chromium.launch({ channel: "msedge", headless: true });
    console.log(`Captures depuis ${server.url} vers ${OUT_DIR}`);
    const stills = await captureStills(browser, server.url);
    const ffmpeg = options["no-gif"] ? null : findFfmpeg();
    if (!options["no-gif"] && !ffmpeg)
      console.warn("ffmpeg introuvable (PATH ou FFMPEG) : GIF non produit.");
    if (ffmpeg) {
      const framesDir = await mkdtemp(path.join(tmpdir(), "flam-screens-"));
      try {
        const frames = await captureJourney(browser, server.url, framesDir);
        const gif = path.join(OUT_DIR, "parcours.gif");
        await buildGif(ffmpeg, frames, framesDir, gif);
        stills.push(gif);
      } finally {
        await rm(framesDir, { recursive: true, force: true });
      }
    }
    for (const file of stills) {
      const { size } = await stat(file);
      console.log(`  ${path.relative(REPO_ROOT, file)}  ${Math.round(size / 1024)} Ko`);
    }
  } finally {
    try {
      await browser?.close();
    } finally {
      await server.close();
    }
  }
  if (problems > 0) {
    console.error(
      `${problems} problème(s) : vérifier les captures avant de les committer.`,
    );
    process.exitCode = 1;
  }
}

await main();

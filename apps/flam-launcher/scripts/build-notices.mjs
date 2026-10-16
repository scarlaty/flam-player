// Génère build/THIRD-PARTY-NOTICES.txt : licences des dépendances tierces embarquées dans
// l'application (crates Rust liées dans flam-launcher.exe, paquets npm compilés dans l'interface).
//
// Sources : `cargo metadata` (graphe de dépendances normales pour Windows, hors dev et build) et
// package-lock.json (entrées non `dev`). Les textes de licence (LICENSE*, COPYING*, NOTICE*) sont
// repris tels quels depuis les sources de chaque paquet ; aucun outil à installer (Node et cargo
// suffisent). Échec si un paquet n'a ni licence déclarée ni texte de licence.
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const launcherDir = join(dirname(fileURLToPath(import.meta.url)), "..");
const tauriDir = join(launcherDir, "src-tauri");
const outFile = join(launcherDir, "build", "THIRD-PARTY-NOTICES.txt");
const LICENSE_FILE = /^(licen[cs]e|copying|notice|unlicense)([-_.].*)?$/i;

function licenseTexts(dir) {
  return readdirSync(dir, { withFileTypes: true })
    .filter((e) => e.isFile() && LICENSE_FILE.test(e.name))
    .map((e) => ({ name: e.name, text: readFileSync(join(dir, e.name), "utf8").trim() }))
    .sort((a, b) => a.name.localeCompare(b.name));
}

function rustPackages() {
  const raw = execFileSync(
    "cargo",
    [
      "metadata",
      "--format-version",
      "1",
      "--locked",
      "--filter-platform",
      "x86_64-pc-windows-msvc",
    ],
    { cwd: tauriDir, maxBuffer: 512 * 1024 * 1024, encoding: "utf8" },
  );
  const meta = JSON.parse(raw);
  const nodes = new Map(meta.resolve.nodes.map((n) => [n.id, n]));
  const seen = new Set();
  const todo = [meta.resolve.root];
  while (todo.length > 0) {
    const id = todo.pop();
    if (seen.has(id)) continue;
    seen.add(id);
    for (const dep of nodes.get(id).deps) {
      // dep_kinds : kind null = dépendance normale ; "dev" et "build" ne sont pas liées au binaire.
      if (dep.dep_kinds.some((k) => k.kind === null)) todo.push(dep.pkg);
    }
  }
  seen.delete(meta.resolve.root);
  return meta.packages
    .filter((p) => seen.has(p.id))
    .map((p) => ({
      ecosystem: "Rust",
      name: p.name,
      version: p.version,
      license: p.license ?? (p.license_file ? `voir ${p.license_file}` : ""),
      url: p.repository ?? p.homepage ?? "",
      dir: dirname(p.manifest_path),
    }));
}

function npmPackages() {
  const lock = JSON.parse(readFileSync(join(launcherDir, "package-lock.json"), "utf8"));
  const out = [];
  for (const [path, entry] of Object.entries(lock.packages)) {
    if (path === "" || entry.dev || entry.devOptional) continue;
    const dir = join(launcherDir, path);
    if (!existsSync(join(dir, "package.json"))) {
      throw new Error(`paquet npm absent (lancer npm ci) : ${path}`);
    }
    const pkg = JSON.parse(readFileSync(join(dir, "package.json"), "utf8"));
    const repo =
      typeof pkg.repository === "string" ? pkg.repository : (pkg.repository?.url ?? "");
    out.push({
      ecosystem: "npm",
      name: pkg.name,
      version: pkg.version,
      license: typeof pkg.license === "string" ? pkg.license : (entry.license ?? ""),
      url: repo || pkg.homepage || "",
      dir,
    });
  }
  return out;
}

const packages = [...rustPackages(), ...npmPackages()].sort(
  (a, b) =>
    a.ecosystem.localeCompare(b.ecosystem) ||
    a.name.localeCompare(b.name) ||
    a.version.localeCompare(b.version),
);

const lines = [
  "Flam Launcher : licences des composants tiers embarqués dans l'application",
  "",
  "Fichier généré par scripts/build-notices.mjs (Cargo.lock, package-lock.json).",
  "Les licences des autres composants livrés (flam-player, SDL2, Lua, LVGL, Python, Pillow...)",
  "sont dans LICENSES.md et le dossier licenses/.",
  "",
  `${packages.length} paquets.`,
  "",
];
const problems = [];
for (const p of packages) {
  const texts = licenseTexts(p.dir);
  if (!p.license && texts.length === 0)
    problems.push(`${p.ecosystem} ${p.name}@${p.version}`);
  lines.push(
    "=".repeat(78),
    `${p.name} ${p.version} (${p.ecosystem})`,
    `Licence : ${p.license || "non déclarée"}`,
    ...(p.url ? [`Source : ${p.url}`] : []),
    "",
  );
  if (texts.length === 0)
    lines.push("(aucun fichier de licence dans les sources du paquet)", "");
  for (const t of texts) lines.push(`--- ${t.name} ---`, t.text, "");
}
if (problems.length > 0) {
  throw new Error(`licence introuvable pour : ${problems.join(", ")}`);
}
mkdirSync(dirname(outFile), { recursive: true });
writeFileSync(outFile, lines.join("\n").replace(/\r\n/g, "\n"), "utf8");
process.stdout.write(`${outFile} : ${packages.length} paquets
`);

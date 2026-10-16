import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, expect, it } from "vitest";
import { isAppError } from "./error";
import {
  getPlayerStatus,
  launchFolderStory,
  launchPlayer,
  onPlayerExited,
  onPlayerStarted,
  type PlayerExited,
  type PlayerSession,
} from "./player";

afterEach(() => clearMocks());

const session: PlayerSession = {
  origin: "library",
  uuid: "u1",
  title: "Mon histoire",
  pid: 42,
  started_at: 1,
};

it("launchPlayer n'envoie que l'identifiant de bibliothèque", async () => {
  const calls: { cmd: string; args: unknown }[] = [];
  mockIPC((cmd, args) => {
    calls.push({ cmd, args });
    return session;
  });
  await expect(launchPlayer("u1")).resolves.toEqual(session);
  expect(calls).toEqual([{ cmd: "player_launch", args: { uuid: "u1" } }]);
});

it("launchFolderStory désigne l'entrée par source et identifiant", async () => {
  const calls: { cmd: string; args: unknown }[] = [];
  mockIPC((cmd, args) => {
    calls.push({ cmd, args });
    return { ...session, origin: "folder" };
  });
  await launchFolderStory("src", "src:abc");
  expect(calls).toEqual([
    { cmd: "player_launch_folder", args: { sourceId: "src", entryId: "src:abc" } },
  ]);
});

it("getPlayerStatus renvoie null sans player", async () => {
  mockIPC(() => null);
  await expect(getPlayerStatus()).resolves.toBeNull();
});

it("une erreur du backend reste une AppError", async () => {
  mockIPC(() => {
    throw { kind: "player_busy", message: "déjà en cours" };
  });
  const error = await launchPlayer("u1").catch((e: unknown) => e);
  expect(isAppError(error)).toBe(true);
  expect(error).toMatchObject({ kind: "player_busy" });
});

it("les écouteurs reçoivent les charges des événements player://", async () => {
  mockIPC(() => null, { shouldMockEvents: true });
  const started: PlayerSession[] = [];
  const exited: PlayerExited[] = [];
  const off = [
    await onPlayerStarted((e) => started.push(e)),
    await onPlayerExited((e) => exited.push(e)),
  ];
  const done: PlayerExited = {
    session,
    code: 7,
    success: false,
    duration_ms: 120,
    stderr: "Erreur de chargement",
  };
  await emit("player://started", session);
  await emit("player://exited", done);
  expect(started).toEqual([session]);
  expect(exited).toEqual([done]);
  off.forEach((unlisten) => unlisten());
});

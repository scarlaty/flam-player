import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, expect, it } from "vitest";
import { isAppError } from "./error";
import {
  cancelInstall,
  createPack,
  listInstalls,
  onInstallDone,
  onInstallError,
  onInstallProgress,
  startInstall,
  type InstallFailed,
  type InstallProgress,
} from "./install";

afterEach(() => clearMocks());

it("startInstall appelle install_start avec la source et l'entrée", async () => {
  const calls: { cmd: string; args: unknown }[] = [];
  mockIPC((cmd, args) => {
    calls.push({ cmd, args });
    return { id: "t1", state: "queued" };
  });
  const task = await startInstall("telmi-store", "telmi-store:abc");
  expect(calls).toEqual([
    {
      cmd: "install_start",
      args: { sourceId: "telmi-store", entryId: "telmi-store:abc" },
    },
  ]);
  expect(task.id).toBe("t1");
});

it("cancelInstall et listInstalls appellent leurs commandes", async () => {
  const calls: { cmd: string; args: unknown }[] = [];
  mockIPC((cmd, args) => {
    calls.push({ cmd, args });
    return cmd === "install_list" ? [] : null;
  });
  await cancelInstall("t1");
  await expect(listInstalls()).resolves.toEqual([]);
  expect(calls.map((c) => c.cmd)).toEqual(["install_cancel", "install_list"]);
  expect(calls[0]?.args).toEqual({ taskId: "t1" });
});

it("une erreur du backend reste une AppError", async () => {
  mockIPC(() => {
    throw { kind: "not_found", message: "histoire inconnue" };
  });
  const error = await startInstall("s", "x").catch((e: unknown) => e);
  expect(isAppError(error)).toBe(true);
  expect(error).toMatchObject({ kind: "not_found" });
});

it("les écouteurs reçoivent les charges des événements install://", async () => {
  mockIPC(() => null, { shouldMockEvents: true });
  const progress: InstallProgress[] = [];
  const failures: InstallFailed[] = [];
  const done: string[] = [];
  const off = [
    await onInstallProgress((e) => progress.push(e)),
    await onInstallError((e) => failures.push(e)),
    await onInstallDone((e) => done.push(e.task_id)),
  ];
  const step: InstallProgress = {
    task_id: "t1",
    state: "converting",
    percent: 37.5,
    received: null,
    total: null,
    phase: "audio",
  };
  await emit("install://progress", step);
  await emit("install://error", {
    task_id: "t1",
    state: "failed",
    error: { kind: "conversion", message: "m", code: "missing_asset" },
  });
  await emit("install://done", { task_id: "t2", entry: {}, warnings: 0 });
  expect(progress).toEqual([step]);
  expect(failures[0]?.error.code).toBe("missing_asset");
  expect(done).toEqual(["t2"]);
  off.forEach((unlisten) => unlisten());
});

it("createPack appelle pack_create avec la composition et la révision du brouillon", async () => {
  const calls: { cmd: string; args: unknown }[] = [];
  mockIPC((cmd, args) => {
    calls.push({ cmd, args });
    return { task: null, report: { valid: false, problems: [] } };
  });
  const composition = {
    metadata: { title: "Pack", subtitle: null, age: 0, description: null },
    cover: { kind: "default" as const },
    menu: [{ kind: "episode" as const, source_id: "s1", episode_id: "e1" }],
  };
  const created = await createPack(composition, 4);
  await createPack(composition);
  expect(calls).toEqual([
    { cmd: "pack_create", args: { composition, draftRevision: 4 } },
    { cmd: "pack_create", args: { composition } },
  ]);
  expect(created.task).toBeNull();
});

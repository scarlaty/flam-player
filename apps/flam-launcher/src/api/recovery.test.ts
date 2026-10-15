import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, expect, it } from "vitest";
import { ackRecovery } from "./recovery";

afterEach(() => clearMocks());

it("ackRecovery acquitte un fichier précis", async () => {
  const calls: { cmd: string; args: unknown }[] = [];
  mockIPC((cmd, args) => {
    calls.push({ cmd, args });
  });
  await ackRecovery("settings.json");
  expect(calls).toEqual([
    { cmd: "config_ack_recovery", args: { file: "settings.json" } },
  ]);
});

it("ackRecovery sans argument acquitte tout", async () => {
  const calls: unknown[] = [];
  mockIPC((_cmd, args) => {
    calls.push(args);
  });
  await ackRecovery();
  expect(calls).toEqual([{ file: null }]);
});

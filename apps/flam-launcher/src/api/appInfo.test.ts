import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, expect, it } from "vitest";
import { getAppInfo } from "./appInfo";

afterEach(() => clearMocks());

it("appelle la commande app_info et renvoie sa réponse", async () => {
  mockIPC((cmd) => {
    expect(cmd).toBe("app_info");
    return { name: "flam-launcher", version: "1.2.3" };
  });

  await expect(getAppInfo()).resolves.toEqual({
    name: "flam-launcher",
    version: "1.2.3",
  });
});

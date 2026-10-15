import { invokeCommand } from "./error";

/** Informations sur l'application (miroir de `AppInfo` côté Rust). */
export interface AppInfo {
  name: string;
  version: string;
}

/** Appelle la commande Tauri `app_info`. */
export function getAppInfo(): Promise<AppInfo> {
  return invokeCommand<AppInfo>("app_info");
}

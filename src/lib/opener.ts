import { openUrl } from "@tauri-apps/plugin-opener";

export const openExternal = (url: string) => openUrl(url);

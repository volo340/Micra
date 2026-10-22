// Envoltorio tipado de los comandos y eventos de Tauri (src-tauri/src/commands.rs).
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import type {
  AppInfo,
  CfFile,
  CfSearchResponse,
  HardwareInfo,
  JavaRuntime,
  MinecraftVersion,
  ModrinthSearchResponse,
  ModrinthVersion,
  NetworkInfo,
  NewServerRequest,
  ProvisionStep,
  ServerConfig,
  ServerEventPayload,
  ServerStatsPayload,
  ServerSummary,
} from "./types";

export const api = {
  appInfo: () => invoke<AppInfo>("app_info"),
  hardwareInfo: () => invoke<HardwareInfo>("hardware_info"),
  networkInfo: () => invoke<NetworkInfo>("network_info"),
  javaRuntimes: () => invoke<JavaRuntime[]>("list_java_runtimes"),
  minecraftVersions: () => invoke<MinecraftVersion[]>("minecraft_versions"),

  listServers: () => invoke<ServerSummary[]>("list_servers"),
  createServer: (request: NewServerRequest) => invoke<ServerConfig>("create_server", { request }),
  startServer: (id: string) => invoke<void>("start_server", { id }),
  stopServer: (id: string) => invoke<void>("stop_server", { id }),
  sendCommand: (id: string, command: string) => invoke<void>("send_command", { id, command }),
  openServerFolder: (id: string) => invoke<void>("open_server_folder", { id }),
  openDataFolder: () => invoke<void>("open_data_folder"),

  searchModrinth: (query: string, gameVersion?: string) =>
    invoke<ModrinthSearchResponse>("search_modrinth", {
      query: { query, kind: "modpack", gameVersion: gameVersion ?? null, limit: 24, offset: 0 },
    }),
  modrinthVersions: (projectId: string) =>
    invoke<ModrinthVersion[]>("modrinth_versions", { projectId, gameVersion: null, loader: null }),
  searchCurseForge: (query: string) =>
    invoke<CfSearchResponse>("search_curseforge", { query, class: "modpack", gameVersion: null, index: 0 }),
  curseForgeFiles: (modId: number) => invoke<CfFile[]>("curseforge_files", { modId, gameVersion: null }),

  openUrl: (url: string) => openUrl(url),
};

export const onServerEvent = (cb: (p: ServerEventPayload) => void): Promise<UnlistenFn> =>
  listen<ServerEventPayload>("server://event", (e) => cb(e.payload));

export const onServerStats = (cb: (p: ServerStatsPayload) => void): Promise<UnlistenFn> =>
  listen<ServerStatsPayload>("server://stats", (e) => cb(e.payload));

export const onProvisionStep = (cb: (p: ProvisionStep) => void): Promise<UnlistenFn> =>
  listen<ProvisionStep>("provision://progress", (e) => cb(e.payload));

/** Los errores de los comandos llegan como texto; cualquier otra cosa se normaliza. */
export function errorMessage(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return "Ha ocurrido un error inesperado";
}

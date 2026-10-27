// Envoltorio tipado de los comandos y eventos de Tauri
// (src-tauri/src/commands/ y src-tauri/src/events.rs).
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import { current } from "../i18n/current";
import type {
  AppErrorPayload,
  AppInfo,
  AppSettings,
  AppUpdate,
  BackupInfo,
  BackupScope,
  CfFile,
  Claim,
  ClaimProgress,
  ContentPage,
  ContentQuery,
  HardwareInfo,
  ImportAnalysis,
  ImportRequest,
  JavaRuntime,
  LivePayload,
  MinecraftVersion,
  ModInfo,
  ModLoader,
  ModRemoteInfo,
  ModrinthVersion,
  NetworkInfo,
  NewServerRequest,
  PackUpdate,
  PlayerAction,
  PlayerLists,
  PropertiesView,
  ProvisionStep,
  ScanReport,
  SearchCursor,
  SearchSource,
  SecretKey,
  ServerConfig,
  ServerEventPayload,
  ServerSettings,
  ServerStatsPayload,
  ServerSummary,
  SettingsSaved,
  SettingsView,
  StatPoint,
  StatsRange,
  TaskPayload,
  UpdateTarget,
} from "./types";

export const api = {
  // La app
  appInfo: () => invoke<AppInfo>("app_info"),
  hardwareInfo: () => invoke<HardwareInfo>("hardware_info"),
  networkInfo: () => invoke<NetworkInfo>("network_info"),
  javaRuntimes: () => invoke<JavaRuntime[]>("list_java_runtimes"),
  openDataFolder: () => invoke<void>("open_data_folder"),
  getSettings: () => invoke<SettingsView>("get_settings"),
  saveSettings: (settings: AppSettings) => invoke<SettingsView>("save_settings", { settings }),
  setSecret: (key: SecretKey, value: string) => invoke<SettingsView>("set_secret", { key, value }),
  deleteSecret: (key: SecretKey) => invoke<SettingsView>("delete_secret", { key }),
  testNotification: () => invoke<void>("test_notification"),
  checkAppUpdate: () => invoke<AppUpdate | null>("check_app_update"),
  installAppUpdate: () => invoke<void>("install_app_update"),

  // Contenido para crear servidores
  minecraftVersions: () => invoke<MinecraftVersion[]>("minecraft_versions"),
  loaderGameVersions: (loader: ModLoader) => invoke<string[]>("loader_game_versions", { loader }),
  /**
   * Mods o modpacks de Modrinth y CurseForge (si hay clave) a la vez. Para un
   * servidor que ya existe, `searchMods`.
   */
  searchContent: (query: ContentQuery) => invoke<ContentPage>("search_content", { query }),
  modrinthVersions: (projectId: string, loaders: string[]) =>
    invoke<ModrinthVersion[]>("modrinth_versions", { projectId, gameVersion: null, loaders }),
  curseForgeFiles: (modId: number) => invoke<CfFile[]>("curseforge_files", { modId, gameVersion: null }),

  // Servidores
  listServers: () => invoke<ServerSummary[]>("list_servers"),
  createServer: (request: NewServerRequest) => invoke<ServerConfig>("create_server", { request }),
  deleteServer: (id: string, deleteBackups: boolean) => invoke<void>("delete_server", { id, deleteBackups }),
  startServer: (id: string) => invoke<void>("start_server", { id }),
  stopServer: (id: string) => invoke<void>("stop_server", { id }),
  restartServer: (id: string) => invoke<void>("restart_server", { id }),
  sendCommand: (id: string, command: string) => invoke<void>("send_command", { id, command }),
  openServerFolder: (id: string) => invoke<void>("open_server_folder", { id }),
  updateServerSettings: (id: string, settings: ServerSettings) =>
    invoke<SettingsSaved>("update_server_settings", { id, settings }),

  // Jugadores
  playerLists: (id: string) => invoke<PlayerLists>("player_lists", { id }),
  playerAction: (id: string, action: PlayerAction, target: string) =>
    invoke<void>("player_action", { id, action, target }),

  // Copias de seguridad
  listBackups: (id: string) => invoke<BackupInfo[]>("list_backups", { id }),
  createBackup: (id: string, scope: BackupScope) => invoke<BackupInfo>("create_backup", { id, scope }),
  restoreBackup: (id: string, name: string) => invoke<void>("restore_backup", { id, name }),
  deleteBackup: (id: string, name: string) => invoke<void>("delete_backup", { id, name }),
  openBackupsFolder: (id: string) => invoke<void>("open_backups_folder", { id }),

  // Mods y modpack
  listMods: (id: string) => invoke<ModInfo[]>("list_mods", { id }),
  setModEnabled: (id: string, fileName: string, enabled: boolean) =>
    invoke<string>("set_mod_enabled", { id, fileName, enabled }),
  removeMod: (id: string, fileName: string) => invoke<void>("remove_mod", { id, fileName }),
  checkModUpdates: (id: string) => invoke<ModRemoteInfo[]>("check_mod_updates", { id }),
  updateMod: (id: string, fileName: string, source: SearchSource, versionId: string) =>
    invoke<string>("update_mod", { id, fileName, source, versionId }),
  /** Mods de Modrinth y CurseForge para este servidor. */
  searchMods: (id: string, query: string, cursor: SearchCursor | null = null) =>
    invoke<ContentPage>("search_mods", { id, query, cursor }),
  installMod: (id: string, source: SearchSource, projectId: string, versionId: string | null = null) =>
    invoke<string[]>("install_mod", { id, source, projectId, versionId }),
  /** Instala un mod que faltaba en el último arranque (el que se encontró para él). */
  installMissingMod: (id: string, modId: string) => invoke<string[]>("install_missing_mod", { id, modId }),
  scanMods: (id: string) => invoke<ScanReport>("scan_mods", { id }),
  quarantineMod: (id: string, path: string) => invoke<void>("quarantine_mod", { id, path }),
  checkPackUpdate: (id: string) => invoke<PackUpdate | null>("check_pack_update", { id }),
  updatePack: (id: string, target: UpdateTarget) => invoke<ServerConfig>("update_pack", { id, target }),
  dismissCrash: (id: string) => invoke<void>("dismiss_crash", { id }),
  openCrashReport: (id: string) => invoke<void>("open_crash_report", { id }),

  // server.properties
  getProperties: (id: string) => invoke<PropertiesView>("get_properties", { id }),
  saveProperties: (id: string, changes: Record<string, string>) =>
    invoke<PropertiesView>("save_properties", { id, changes }),

  // Estadísticas
  serverStats: (id: string, range: StatsRange) => invoke<StatPoint[]>("server_stats", { id, range }),

  // Red
  publicIp: () => invoke<string>("public_ip"),
  checkReachable: (host: string, port: number) => invoke<boolean>("check_reachable", { host, port }),
  playitClaimStart: () => invoke<Claim>("playit_claim_start"),
  playitClaimPoll: (code: string) => invoke<ClaimProgress>("playit_claim_poll", { code }),

  // Importar
  pickFolder: () => invoke<string | null>("pick_folder"),
  analyzeImport: (path: string) => invoke<ImportAnalysis>("analyze_import", { path }),
  importServer: (request: ImportRequest) => invoke<ServerConfig>("import_server", { request }),

  // Mapa web
  enableMap: (id: string, acceptDownload: boolean) => invoke<ServerConfig>("enable_map", { id, acceptDownload }),
  disableMap: (id: string) => invoke<ServerConfig>("disable_map", { id }),
  acceptMapDownload: (id: string) => invoke<ServerConfig>("accept_map_download", { id }),
  openMap: (id: string) => invoke<void>("open_map", { id }),

  openUrl: (url: string) => openUrl(url),
};

export const onServerEvent = (cb: (p: ServerEventPayload) => void): Promise<UnlistenFn> =>
  listen<ServerEventPayload>("server://event", (e) => cb(e.payload));

export const onServerStats = (cb: (p: ServerStatsPayload) => void): Promise<UnlistenFn> =>
  listen<ServerStatsPayload>("server://stats", (e) => cb(e.payload));

export const onServerLive = (cb: (p: LivePayload) => void): Promise<UnlistenFn> =>
  listen<LivePayload>("server://live", (e) => cb(e.payload));

export const onTaskProgress = (cb: (p: TaskPayload) => void): Promise<UnlistenFn> =>
  listen<TaskPayload>("task://progress", (e) => cb(e.payload));

export const onAppError = (cb: (p: AppErrorPayload) => void): Promise<UnlistenFn> =>
  listen<AppErrorPayload>("app://error", (e) => cb(e.payload));

export const onProvisionStep = (cb: (p: ProvisionStep) => void): Promise<UnlistenFn> =>
  listen<ProvisionStep>("provision://progress", (e) => cb(e.payload));

/** Los errores de los comandos llegan como texto; cualquier otra cosa se normaliza. */
export function errorMessage(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return current().common.unexpectedError;
}

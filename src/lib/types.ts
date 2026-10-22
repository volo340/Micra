// Espejo de los tipos que serializa el backend (micra-core / src-tauri).
// `Option<T>` de Rust llega como `T | null`.

export type Lang = "es" | "en" | "pt" | "fr" | "de";

export type ServerStatus = "starting" | "running" | "stopping" | "stopped" | "crashed";

export type Loader =
  | { kind: "vanilla" }
  | { kind: "fabric"; loaderVersion: string }
  | { kind: "quilt"; loaderVersion: string }
  | { kind: "forge"; version: string }
  | { kind: "neoForge"; version: string };

export interface MemoryConfig {
  minMb: number;
  maxMb: number;
}

export interface CpuConfig {
  cores: number[] | null;
}

export interface BackupPolicy {
  enabled: boolean;
  intervalHours: number;
  keep: number;
}

export interface SchedulePolicy {
  /** Hora local "HH:MM". */
  dailyRestart: string | null;
  restartEveryHours: number | null;
  idleShutdownMinutes: number | null;
}

export interface NetworkPolicy {
  upnp: boolean;
  tunnel: boolean;
}

export interface GuardPolicy {
  enabled: boolean;
  maxAttempts: number;
  windowSecs: number;
  /** 0 = permanente. */
  banMinutes: number;
  /** 0 = sin límite. */
  maxPerIp: number;
}

export interface SecurityPolicy {
  guard: GuardPolicy;
  sandbox: boolean;
  lowIntegrity: boolean;
}

export interface MapConfig {
  enabled: boolean;
  port: number;
  acceptedDownload: boolean;
}

export type ContentSource =
  | { kind: "modrinth"; projectId: string; versionId: string }
  | { kind: "curseForge"; modId: number; fileId: number };

export type LaunchTarget = { kind: "jar"; path: string } | { kind: "argsFile"; path: string };

export interface ServerConfig {
  id: string;
  name: string;
  mcVersion: string;
  loader: Loader;
  javaMajor: number;
  memory: MemoryConfig;
  cpu: CpuConfig;
  port: number;
  eulaAccepted: boolean;
  launch: LaunchTarget;
  useAikarFlags: boolean;
  extraJvmArgs: string[];
  backups: BackupPolicy;
  schedule: SchedulePolicy;
  network: NetworkPolicy;
  security: SecurityPolicy;
  map: MapConfig;
  source: ContentSource | null;
  pendingOwner: string | null;
}

export type WanKind = "public" | "cgnat" | "private";

export interface PortMapping {
  port: number;
  localIp: string;
  externalIp: string | null;
  wan: WanKind | null;
  permanent: boolean;
}

export type UpnpInfo = { kind: "opening" } | { kind: "open"; mapping: PortMapping } | { kind: "failed"; error: string };

export type TunnelInfo =
  | { kind: "starting" }
  | { kind: "ready"; address: string }
  | { kind: "pending" }
  | { kind: "disabled"; reason: string }
  | { kind: "failed"; error: string };

export type SandboxStatus =
  | { kind: "disabled" }
  | { kind: "landlock"; abi: number }
  | { kind: "jobObject"; lowIntegrity: boolean }
  | { kind: "unavailable"; reason: string };

export type UpdateTarget = { kind: "modrinth"; versionId: string } | { kind: "curseForge"; fileId: number };

export interface PackUpdate {
  current: string | null;
  latest: string;
  target: UpdateTarget;
}

/** Lo que el panel muestra de cada servidor además de su estado. */
export interface LiveInfo {
  tps: number | null;
  upnp: UpnpInfo | null;
  tunnel: TunnelInfo | null;
  restartAt: string | null;
  packUpdate: PackUpdate | null;
  modUpdates: number;
  sandbox: SandboxStatus | null;
  mapNeedsAccept: boolean;
  lastBackupError: string | null;
}

export interface ServerSummary {
  config: ServerConfig;
  status: ServerStatus;
  players: string[];
  dir: string;
  live: LiveInfo;
  /** Hay una operación larga en curso (copia, restauración, actualización). */
  busy: boolean;
  startedAt: string | null;
}

/** Lo que se guarda desde las pestañas de ajustes del servidor. */
export interface ServerSettings {
  memory: MemoryConfig;
  cpu: CpuConfig;
  port: number;
  useAikarFlags: boolean;
  backups: BackupPolicy;
  schedule: SchedulePolicy;
  network: NetworkPolicy;
  security: SecurityPolicy;
  mapPort: number;
}

export interface SettingsSaved {
  config: ServerConfig;
  restartNeeded: boolean;
}

export interface HardwareInfo {
  logicalCpus: number;
  totalMemoryMb: number;
  recommendedMaxMb: number;
}

export interface AppInfo {
  version: string;
  dataDir: string;
  curseforgeEnabled: boolean;
  language: Lang;
  platform: "windows" | "linux";
}

export interface NetworkInfo {
  localIp: string | null;
}

export interface JavaRuntime {
  major: number;
  releaseName: string;
  home: string;
  javaBin: string;
}

// ---------- Ajustes de la app ----------

export interface NotificationSettings {
  desktop: boolean;
  discord: boolean;
  playerJoin: boolean;
  playerLeave: boolean;
  serverReady: boolean;
  serverCrash: boolean;
  security: boolean;
  backupFailed: boolean;
}

export interface AppSettings {
  /** `null`: el idioma del sistema. */
  language: Lang | null;
  notifications: NotificationSettings;
  updates: { autoCheck: boolean };
}

export type SecretKey = "discordWebhook" | "curseforgeApiKey" | "playitAgent";

export interface SettingsView {
  settings: AppSettings;
  /** Idioma en uso (el elegido o el del sistema). */
  language: Lang;
  secrets: Record<SecretKey, boolean>;
  secretBackend: "keyring" | "file";
}

export interface AppUpdate {
  version: string;
  notes: string | null;
  date: string | null;
  /** `false` en Linux con .deb o .rpm: se actualiza con el instalador. */
  canInstall: boolean;
}

// ---------- Eventos ----------

export type RejectReason = "notWhitelisted" | "banned";

export type ServerEvent =
  | { type: "log"; stream: "stdout" | "stderr"; line: string }
  | { type: "status"; status: ServerStatus }
  | { type: "playerJoined"; name: string }
  | { type: "playerLeft"; name: string }
  | { type: "joinRejected"; name: string; reason: RejectReason }
  | { type: "exited"; code: number | null };

export interface ServerEventPayload {
  serverId: string;
  event: ServerEvent;
}

export interface ResourceSample {
  cpuPercent: number;
  memoryMb: number;
}

export interface ServerStatsPayload {
  serverId: string;
  sample: ResourceSample;
  playerCount: number;
  tps: number | null;
}

export interface LivePayload {
  serverId: string;
  live: LiveInfo;
}

export type TaskKind = "backup" | "restore" | "scan" | "import";

export interface TaskPayload {
  serverId: string | null;
  task: TaskKind;
  done: number;
  total: number;
}

export interface AppErrorPayload {
  title: string;
  message: string;
}

// ---------- Crear e importar ----------

export type ServerSource =
  | { kind: "vanilla"; mcVersion: string }
  | { kind: "fabric"; mcVersion: string; loaderVersion: string | null }
  | { kind: "modrinth"; versionId: string }
  | { kind: "curseForge"; modId: number; fileId: number };

export interface NewServerRequest {
  name: string;
  source: ServerSource;
  memory: MemoryConfig;
  cpu: CpuConfig;
  port: number;
  eulaAccepted: boolean;
  /** Cuenta de Minecraft de quien lo crea: lista blanca y operador. */
  owner: string | null;
}

export type ProvisionStep =
  | { step: "fetchingContent" }
  | { step: "downloadingFiles"; done: number; total: number }
  | { step: "installingJava"; major: number }
  | { step: "downloadingJava"; downloaded: number; total: number | null }
  | { step: "installingLoader" }
  | { step: "finalizing" };

export interface ImportAnalysis {
  path: string;
  suggestedName: string;
  mcVersion: string | null;
  loader: Loader;
  launch: LaunchTarget | null;
  hasInstaller: boolean;
  size: number;
  hasWorld: boolean;
  port: number | null;
  onlineMode: boolean;
  whitelist: boolean;
  modCount: number;
}

export interface ImportRequest {
  path: string;
  name: string;
  mcVersion: string | null;
  memory: MemoryConfig;
  cpu: CpuConfig;
  port: number;
  eulaAccepted: boolean;
}

export interface MinecraftVersion {
  id: string;
  type: string;
}

// ---------- Jugadores ----------

export interface WhitelistEntry {
  uuid: string;
  name: string;
}

export interface OpEntry {
  uuid: string;
  name: string;
  level: number;
  bypassesPlayerLimit: boolean;
}

export interface BannedPlayer {
  uuid: string;
  name: string;
  created: string;
  source: string;
  /** "forever" o una fecha. */
  expires: string;
  reason: string;
}

export interface BannedIp {
  ip: string;
  created: string;
  source: string;
  expires: string;
  reason: string;
}

export interface KnownPlayer {
  name: string;
  uuid: string;
}

export interface PlayerLists {
  whitelist: WhitelistEntry[];
  ops: OpEntry[];
  bannedPlayers: BannedPlayer[];
  bannedIps: BannedIp[];
  known: KnownPlayer[];
}

export type PlayerAction =
  | { kind: "whitelistAdd" }
  | { kind: "whitelistRemove" }
  | { kind: "op" }
  | { kind: "deop" }
  | { kind: "ban"; reason: string }
  | { kind: "pardon" }
  | { kind: "banIp"; reason: string }
  | { kind: "pardonIp" }
  | { kind: "kick"; reason: string };

// ---------- Copias ----------

export type BackupKind = "manual" | "auto" | "preRestore" | "preUpdate";
export type BackupScope = "world" | "full";

export interface BackupInfo {
  fileName: string;
  kind: BackupKind;
  scope: BackupScope;
  createdAt: string;
  size: number;
  mcVersion: string;
}

// ---------- Mods ----------

export interface ModInfo {
  fileName: string;
  enabled: boolean;
  size: number;
  sha1: string;
  modId: string | null;
  name: string | null;
  version: string | null;
  fromPack: boolean;
}

export interface ModUpdate {
  versionId: string;
  versionNumber: string;
  fileName: string;
}

export interface ModRemoteInfo {
  sha1: string;
  projectId: string;
  title: string | null;
  iconUrl: string | null;
  versionNumber: string;
  update: ModUpdate | null;
}

export type Severity = "suspicious" | "critical";

export interface Finding {
  severity: Severity;
  rule: string;
  location: string;
}

export interface FileReport {
  path: string;
  sha1: string;
  findings: Finding[];
}

export interface ScanReport {
  scanned: number;
  files: FileReport[];
  host: Finding[];
}

// ---------- Propiedades, estadísticas y red ----------

export interface PropertiesView {
  values: Record<string, string>;
  exists: boolean;
}

export type StatsRange = "hour" | "day" | "week" | "month";

export interface StatPoint {
  /** Segundos desde 1970 (UTC). */
  ts: number;
  cpu: number;
  ramMb: number;
  players: number;
  tps: number | null;
}

export interface Claim {
  code: string;
  url: string;
}

export type ClaimStatus = "waitingForUserVisit" | "waitingForUser" | "userAccepted" | "userRejected";

export interface ClaimProgress {
  status: ClaimStatus;
  linked: boolean;
}

// ---------- Modrinth (la API usa snake_case) ----------

export interface ModrinthHit {
  project_id: string;
  slug: string;
  title: string;
  description: string;
  author: string;
  downloads: number;
  icon_url: string | null;
  categories: string[];
  versions: string[];
}

export interface ModrinthSearchResponse {
  hits: ModrinthHit[];
  total_hits: number;
  offset: number;
  limit: number;
}

export interface ModrinthVersion {
  id: string;
  name: string;
  version_number: string;
  version_type: string;
  game_versions: string[];
  loaders: string[];
  date_published: string;
}

// ---------- CurseForge (camelCase) ----------

export interface CfFile {
  id: number;
  modId: number;
  displayName: string;
  fileName: string;
  gameVersions: string[];
  serverPackFileId: number | null;
  isServerPack: boolean | null;
}

export interface CfMod {
  id: number;
  name: string;
  slug: string;
  summary: string;
  downloadCount: number;
  logo: { thumbnailUrl: string } | null;
  latestFiles: CfFile[];
}

export interface CfSearchResponse {
  data: CfMod[];
}

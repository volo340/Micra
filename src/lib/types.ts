// Espejo de los tipos que serializa el backend (micra-core / src-tauri).

export type ServerStatus = "starting" | "running" | "stopping" | "stopped" | "crashed";

export type Loader =
  | { kind: "vanilla" }
  | { kind: "fabric"; loaderVersion: string }
  | { kind: "quilt"; loaderVersion: string }
  | { kind: "forge"; version: string }
  | { kind: "neoForge"; version: string };

export interface ServerConfig {
  id: string;
  name: string;
  mcVersion: string;
  loader: Loader;
  javaMajor: number;
  memory: { minMb: number; maxMb: number };
  cpu: { cores: number[] | null };
  port: number;
  eulaAccepted: boolean;
}

export interface ServerSummary {
  config: ServerConfig;
  status: ServerStatus;
  players: string[];
  dir: string;
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

export type ServerEvent =
  | { type: "log"; stream: "stdout" | "stderr"; line: string }
  | { type: "status"; status: ServerStatus }
  | { type: "playerJoined"; name: string }
  | { type: "playerLeft"; name: string }
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
}

export type ServerSource =
  | { kind: "vanilla"; mcVersion: string }
  | { kind: "fabric"; mcVersion: string; loaderVersion: string | null }
  | { kind: "modrinth"; versionId: string }
  | { kind: "curseForge"; modId: number; fileId: number };

export interface NewServerRequest {
  name: string;
  source: ServerSource;
  memory: { minMb: number; maxMb: number };
  cpu: { cores: number[] | null };
  port: number;
  eulaAccepted: boolean;
}

export type ProvisionStep =
  | { step: "fetchingContent" }
  | { step: "downloadingFiles"; done: number; total: number }
  | { step: "installingJava"; major: number }
  | { step: "downloadingJava"; downloaded: number; total: number | null }
  | { step: "installingLoader" }
  | { step: "finalizing" };

export interface MinecraftVersion {
  id: string;
  type: string;
}

// Modrinth (la API usa snake_case).
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

// CurseForge (camelCase).
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

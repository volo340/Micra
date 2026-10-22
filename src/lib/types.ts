// Espejo de los tipos serializados por micra-core (camelCase).

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
}

export interface HardwareInfo {
  logicalCpus: number;
  totalMemoryMb: number;
  recommendedMaxMb: number;
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

export interface ServerStatsPayload {
  serverId: string;
  sample: { cpuPercent: number; memoryMb: number };
  playerCount: number;
}

export type ServerSource =
  | { kind: "vanilla"; mcVersion: string }
  | { kind: "fabric"; mcVersion: string; loaderVersion: string }
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

export interface ModrinthHit {
  project_id: string;
  slug: string;
  title: string;
  description: string;
  author: string;
  downloads: number;
  icon_url: string | null;
  versions: string[];
}

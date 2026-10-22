import type { HardwareInfo, NewServerRequest } from "../../lib/types";

export type Kind = "vanilla" | "fabric" | "modrinth" | "curseforge";

export interface PackChoice {
  source: "modrinth" | "curseforge";
  projectId: string;
  title: string;
  iconUrl: string | null;
  versionId: string | null;
  versionLabel: string | null;
}

export interface Draft {
  kind: Kind;
  mcVersion: string | null;
  pack: PackChoice | null;
  name: string;
  nameTouched: boolean;
  maxMb: number;
  allCores: boolean;
  cores: number[];
  port: number;
  eula: boolean;
}

export const isModpack = (kind: Kind) => kind === "modrinth" || kind === "curseforge";

/** Memoria por defecto: 4 GB para vanilla/Fabric, 6 GB para modpacks, sin pasar del máximo recomendado. */
export function defaultMemory(kind: Kind, hardware: HardwareInfo | null): number {
  const wanted = isModpack(kind) ? 6144 : 4096;
  const ceiling = maxMemory(hardware);
  return Math.max(1024, Math.min(wanted, ceiling));
}

export function maxMemory(hardware: HardwareInfo | null): number {
  const recommended = hardware?.recommendedMaxMb ?? 8192;
  return Math.max(2048, Math.floor(recommended / 512) * 512);
}

export function newDraft(hardware: HardwareInfo | null): Draft {
  return {
    kind: "vanilla",
    mcVersion: null,
    pack: null,
    name: "Mi servidor",
    nameTouched: false,
    maxMb: defaultMemory("vanilla", hardware),
    allCores: true,
    cores: [],
    port: 25565,
    eula: false,
  };
}

export const validPort = (port: number) => Number.isInteger(port) && port >= 1024 && port <= 65535;

export function toRequest(d: Draft): NewServerRequest {
  let source: NewServerRequest["source"];
  switch (d.kind) {
    case "vanilla":
      source = { kind: "vanilla", mcVersion: d.mcVersion ?? "" };
      break;
    case "fabric":
      source = { kind: "fabric", mcVersion: d.mcVersion ?? "", loaderVersion: null };
      break;
    case "modrinth":
      source = { kind: "modrinth", versionId: d.pack?.versionId ?? "" };
      break;
    case "curseforge":
      source = { kind: "curseForge", modId: Number(d.pack?.projectId), fileId: Number(d.pack?.versionId) };
      break;
  }
  return {
    name: d.name.trim(),
    source,
    // Un mínimo moderado deja RAM libre al resto del equipo mientras el servidor no la necesita.
    memory: { minMb: Math.min(2048, d.maxMb), maxMb: d.maxMb },
    cpu: { cores: d.allCores ? null : [...d.cores].sort((a, b) => a - b) },
    port: d.port,
    eulaAccepted: d.eula,
  };
}

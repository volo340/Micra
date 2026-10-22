import { maxMemory } from "../../components/ResourceFields";
import type { HardwareInfo, ImportAnalysis, ImportRequest, NewServerRequest } from "../../lib/types";

export type Kind = "vanilla" | "fabric" | "modrinth" | "curseforge" | "import";

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
  /** Carpeta elegida para importar y lo que Micra entendió de ella. */
  analysis: ImportAnalysis | null;
  name: string;
  nameTouched: boolean;
  maxMb: number;
  allCores: boolean;
  cores: number[];
  port: number;
  /** Cuenta de Minecraft de quien lo crea (lista blanca y operador). */
  owner: string;
  eula: boolean;
}

export const isModpack = (kind: Kind) => kind === "modrinth" || kind === "curseforge";

/** Memoria por defecto: 4 GB para vanilla/Fabric, 6 GB para modpacks, sin pasar del máximo recomendado. */
export function defaultMemory(kind: Kind, hardware: HardwareInfo | null, modded = false): number {
  const wanted = isModpack(kind) || modded ? 6144 : 4096;
  return Math.max(1024, Math.min(wanted, maxMemory(hardware)));
}

/** El primer puerto libre desde el de siempre (25565). */
export function freePort(used: number[], from = 25565): number {
  let port = from;
  while (used.includes(port)) port++;
  return port;
}

export function newDraft(hardware: HardwareInfo | null, name: string, port: number): Draft {
  return {
    kind: "vanilla",
    mcVersion: null,
    pack: null,
    analysis: null,
    name,
    nameTouched: false,
    maxMb: defaultMemory("vanilla", hardware),
    allCores: true,
    cores: [],
    port,
    owner: "",
    eula: false,
  };
}

const memory = (d: Draft) => ({ minMb: Math.min(2048, d.maxMb), maxMb: d.maxMb });
const cpu = (d: Draft) => ({ cores: d.allCores ? null : [...d.cores].sort((a, b) => a - b) });

export function toRequest(d: Draft): NewServerRequest {
  let source: NewServerRequest["source"];
  switch (d.kind) {
    case "fabric":
      source = { kind: "fabric", mcVersion: d.mcVersion ?? "", loaderVersion: null };
      break;
    case "modrinth":
      source = { kind: "modrinth", versionId: d.pack?.versionId ?? "" };
      break;
    case "curseforge":
      source = { kind: "curseForge", modId: Number(d.pack?.projectId), fileId: Number(d.pack?.versionId) };
      break;
    default:
      source = { kind: "vanilla", mcVersion: d.mcVersion ?? "" };
  }
  return {
    name: d.name.trim(),
    source,
    // Un mínimo moderado deja RAM libre al resto del equipo mientras el servidor no la necesita.
    memory: memory(d),
    cpu: cpu(d),
    port: d.port,
    eulaAccepted: d.eula,
    owner: d.owner.trim() || null,
  };
}

export function toImportRequest(d: Draft): ImportRequest {
  return {
    path: d.analysis?.path ?? "",
    name: d.name.trim(),
    mcVersion: d.analysis?.mcVersion ?? d.mcVersion,
    memory: memory(d),
    cpu: cpu(d),
    port: d.port,
    eulaAccepted: d.eula,
  };
}

import { maxMemory } from "../../components/ResourceFields";
import type {
  HardwareInfo,
  ImportAnalysis,
  ImportRequest,
  ModLoader,
  NewServerRequest,
  SearchSource,
} from "../../lib/types";

/** Tipo de servidor: la familia Forge incluye NeoForge. */
export type Kind = "vanilla" | "fabric" | "forge" | "import";
/** Con Fabric o Forge: elegir la versión y los mods, o usar un modpack hecho. */
export type ContentMode = "mods" | "modpack";
export type ForgeLoader = "neoForge" | "forge";

export interface PackChoice {
  source: SearchSource;
  projectId: string;
  title: string;
  iconUrl: string | null;
  versionId: string | null;
  versionLabel: string | null;
}

/** Mod de Modrinth o CurseForge que se instala al terminar de crear el servidor. */
export interface ModChoice {
  source: SearchSource;
  projectId: string;
  title: string;
  iconUrl: string | null;
}

export interface Draft {
  kind: Kind;
  content: ContentMode;
  forgeLoader: ForgeLoader;
  mcVersion: string | null;
  pack: PackChoice | null;
  mods: ModChoice[];
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

/** Fabric y Forge cargan mods: se eligen sueltos o con un modpack. */
export const usesLoader = (kind: Kind) => kind === "fabric" || kind === "forge";

export const isModpack = (d: Pick<Draft, "kind" | "content">) => usesLoader(d.kind) && d.content === "modpack";

/** Loader del servidor cuando se eligen los mods (con modpack lo decide el pack). */
export function modLoader(d: Pick<Draft, "kind" | "forgeLoader">): ModLoader | null {
  if (d.kind === "fabric") return "fabric";
  if (d.kind === "forge") return d.forgeLoader;
  return null;
}

/** Loaders (nombres de Modrinth) cuyos mods o modpacks valen para este servidor. */
export function modrinthLoaders(d: Pick<Draft, "kind" | "content" | "forgeLoader" | "mcVersion">): string[] {
  if (d.kind === "fabric") return ["fabric"];
  if (d.kind !== "forge") return [];
  if (d.content === "modpack") return ["forge", "neoforge"];
  if (d.forgeLoader === "forge") return ["forge"];
  // NeoForge para 1.20.1 carga también los mods de Forge (como el servidor).
  return d.mcVersion === "1.20.1" ? ["neoforge", "forge"] : ["neoforge"];
}

/** El mismo mod en el asistente y en las búsquedas: fuente e id. */
export const modKey = (m: { source: SearchSource; projectId: string }) => `${m.source}:${m.projectId}`;

/** Forge y los modpacks piden más memoria que vanilla o Fabric. */
export const isHeavy = (d: Pick<Draft, "kind" | "content">) => d.kind === "forge" || isModpack(d);

/** Memoria por defecto: 4 GB para vanilla y Fabric, 6 GB para Forge y modpacks, sin pasar del máximo recomendado. */
export function defaultMemory(hardware: HardwareInfo | null, heavy: boolean): number {
  const wanted = heavy ? 6144 : 4096;
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
    content: "mods",
    forgeLoader: "neoForge",
    mcVersion: null,
    pack: null,
    mods: [],
    analysis: null,
    name,
    nameTouched: false,
    maxMb: defaultMemory(hardware, false),
    allCores: true,
    cores: [],
    port,
    owner: "",
    eula: false,
  };
}

const memory = (d: Draft) => ({ minMb: Math.min(2048, d.maxMb), maxMb: d.maxMb });
const cpu = (d: Draft) => ({ cores: d.allCores ? null : [...d.cores].sort((a, b) => a - b) });

function sourceOf(d: Draft): NewServerRequest["source"] {
  const mcVersion = d.mcVersion ?? "";
  if (isModpack(d)) {
    return d.pack?.source === "curseforge"
      ? { kind: "curseForge", modId: Number(d.pack.projectId), fileId: Number(d.pack.versionId) }
      : { kind: "modrinth", versionId: d.pack?.versionId ?? "" };
  }
  switch (modLoader(d)) {
    case "fabric":
      return { kind: "fabric", mcVersion, loaderVersion: null };
    case "forge":
      return { kind: "forge", mcVersion, version: null };
    case "neoForge":
      return { kind: "neoForge", mcVersion, version: null };
    default:
      return { kind: "vanilla", mcVersion };
  }
}

export function toRequest(d: Draft): NewServerRequest {
  return {
    name: d.name.trim(),
    source: sourceOf(d),
    // Un mínimo moderado deja RAM libre al resto del equipo mientras el servidor no la necesita.
    memory: memory(d),
    cpu: cpu(d),
    port: d.port,
    eulaAccepted: d.eula,
    owner: d.owner.trim() || null,
  };
}

/** Mods que hay que instalar después de crear el servidor. */
export const modsToInstall = (d: Draft) => (usesLoader(d.kind) && d.content === "mods" ? d.mods : []);

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

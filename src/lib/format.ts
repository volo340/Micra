const decimal = new Intl.NumberFormat("es-ES", { maximumFractionDigits: 1 });
const compact = new Intl.NumberFormat("es-ES", { notation: "compact", maximumFractionDigits: 1 });

export const formatGb = (mb: number) => `${decimal.format(mb / 1024)} GB`;

export const formatCompact = (n: number) => compact.format(n);

export function formatBytes(bytes: number): string {
  if (bytes < 1024 * 1024) return `${decimal.format(bytes / 1024)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${decimal.format(bytes / 1024 / 1024)} MB`;
  return `${decimal.format(bytes / 1024 / 1024 / 1024)} GB`;
}

export const formatPercent = (n: number) => `${Math.round(n)} %`;

const LOADER_NAMES: Record<string, string> = {
  vanilla: "Vanilla",
  fabric: "Fabric",
  quilt: "Quilt",
  forge: "Forge",
  neoForge: "NeoForge",
  neoforge: "NeoForge",
};

/** Nombre legible de un loader, venga del backend ("neoForge") o de Modrinth ("neoforge"). */
export const loaderName = (kind: string) => LOADER_NAMES[kind] ?? kind;

export function loaderLabel(loader: { kind: string; version?: string; loaderVersion?: string }): string {
  const name = loaderName(loader.kind);
  const version = loader.version ?? loader.loaderVersion;
  return version ? `${name} ${version}` : name;
}

/** Rango compacto de núcleos: [0,1,2,3,6] -> "0–3, 6". */
export function formatCores(cores: number[]): string {
  const sorted = [...cores].sort((a, b) => a - b);
  const parts: string[] = [];
  let start = sorted[0];
  let prev = sorted[0];
  for (const c of [...sorted.slice(1), Number.NaN]) {
    if (c === prev + 1) {
      prev = c;
      continue;
    }
    parts.push(start === prev ? `${start}` : `${start}–${prev}`);
    start = c;
    prev = c;
  }
  return parts.join(", ");
}

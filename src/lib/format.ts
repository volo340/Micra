// Formato de cifras, fechas y nombres. Los formateadores siguen el idioma de
// la interfaz: `I18nProvider` llama a `setFormatLocale` al cambiarlo.

let decimal = new Intl.NumberFormat("es-ES", { maximumFractionDigits: 1 });
let oneDecimal = new Intl.NumberFormat("es-ES", { minimumFractionDigits: 1, maximumFractionDigits: 1 });
let compact = new Intl.NumberFormat("es-ES", { notation: "compact", maximumFractionDigits: 1 });
let percent = new Intl.NumberFormat("es-ES", { style: "percent", maximumFractionDigits: 0 });
let dateTime = new Intl.DateTimeFormat("es-ES", { dateStyle: "medium", timeStyle: "short" });
let shortDate = new Intl.DateTimeFormat("es-ES", { day: "numeric", month: "short" });
let clock = new Intl.DateTimeFormat("es-ES", { hour: "2-digit", minute: "2-digit" });
let relative = new Intl.RelativeTimeFormat("es-ES", { numeric: "auto" });
// "GB" en casi todos los idiomas, "Go" en francés.
const sizeUnit = (tag: string, unit: string) =>
  new Intl.NumberFormat(tag, { style: "unit", unit, maximumFractionDigits: 1 });
let kb = sizeUnit("es-ES", "kilobyte");
let mb = sizeUnit("es-ES", "megabyte");
let gb = sizeUnit("es-ES", "gigabyte");

export function setFormatLocale(tag: string) {
  decimal = new Intl.NumberFormat(tag, { maximumFractionDigits: 1 });
  oneDecimal = new Intl.NumberFormat(tag, { minimumFractionDigits: 1, maximumFractionDigits: 1 });
  compact = new Intl.NumberFormat(tag, { notation: "compact", maximumFractionDigits: 1 });
  percent = new Intl.NumberFormat(tag, { style: "percent", maximumFractionDigits: 0 });
  dateTime = new Intl.DateTimeFormat(tag, { dateStyle: "medium", timeStyle: "short" });
  shortDate = new Intl.DateTimeFormat(tag, { day: "numeric", month: "short" });
  clock = new Intl.DateTimeFormat(tag, { hour: "2-digit", minute: "2-digit" });
  relative = new Intl.RelativeTimeFormat(tag, { numeric: "auto" });
  kb = sizeUnit(tag, "kilobyte");
  mb = sizeUnit(tag, "megabyte");
  gb = sizeUnit(tag, "gigabyte");
}

export const formatGb = (megabytes: number) => gb.format(megabytes / 1024);

export const formatCompact = (n: number) => compact.format(n);

export const formatNumber = (n: number) => decimal.format(n);

export function formatBytes(bytes: number): string {
  if (bytes < 1024 * 1024) return kb.format(bytes / 1024);
  if (bytes < 1024 * 1024 * 1024) return mb.format(bytes / 1024 / 1024);
  return gb.format(bytes / 1024 / 1024 / 1024);
}

export const formatPercent = (n: number) => percent.format(n / 100);

/** TPS siempre con un decimal: "19,8". */
export const formatTps = (n: number) => oneDecimal.format(n);

/** Una fecha como `Date`, texto ISO o milisegundos. */
type DateLike = Date | string | number;

export const formatDateTime = (date: DateLike) => dateTime.format(new Date(date));

export const formatShortDate = (date: DateLike) => shortDate.format(new Date(date));

export const formatClock = (date: DateLike) => clock.format(new Date(date));

/** "hace 5 minutos", "ayer"... */
export function formatRelative(date: DateLike): string {
  const seconds = (new Date(date).getTime() - Date.now()) / 1000;
  const abs = Math.abs(seconds);
  if (abs < 60) return relative.format(Math.round(seconds), "second");
  if (abs < 3600) return relative.format(Math.round(seconds / 60), "minute");
  if (abs < 86400) return relative.format(Math.round(seconds / 3600), "hour");
  return relative.format(Math.round(seconds / 86400), "day");
}

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

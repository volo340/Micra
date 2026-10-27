/**
 * Una búsqueda de mods que otra vista pide abrir (la tarjeta del error, con
 * el mod que falta): el buscador de la pestaña Mods la recoge al abrirse.
 */
const pending = new Map<string, string>();

export function requestModSearch(serverId: string, query: string) {
  pending.set(serverId, query);
}

/** La búsqueda pedida para este servidor, sin olvidarla todavía. */
export function peekModSearch(serverId: string): string {
  return pending.get(serverId) ?? "";
}

export function clearModSearch(serverId: string) {
  pending.delete(serverId);
}

// Diccionario activo para el código que no es un componente (mensajes de
// error genéricos, por ejemplo). Los componentes usan `useT()`.
import { es, type Dict } from "./es";

let active: Dict = es;

export const current = () => active;

export function setCurrent(dict: Dict) {
  active = dict;
}

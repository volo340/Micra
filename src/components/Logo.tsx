import logoUrl from "../assets/logo.svg";

/** Bloque de Micra (hierba + servidor). `size` es la altura en píxeles. */
export function Logo({ size = 32, className }: { size?: number; className?: string }) {
  return (
    <img src={logoUrl} alt="" className={className} height={size} width={Math.round(size * 0.87)} draggable={false} />
  );
}

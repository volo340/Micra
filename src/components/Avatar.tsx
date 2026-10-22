import { useState } from "react";

/** Cabeza del skin del jugador; si no hay red o el nombre no existe, su inicial. */
export function Avatar({ name, size }: { name: string; size?: number }) {
  const [failed, setFailed] = useState(false);
  const style = size ? { width: size, height: size } : undefined;
  return (
    <span className="avatar" style={style} title={name}>
      {name.slice(0, 1).toUpperCase()}
      {!failed && (
        <img
          src={`https://mc-heads.net/avatar/${encodeURIComponent(name)}/64`}
          alt=""
          loading="lazy"
          onError={() => setFailed(true)}
        />
      )}
    </span>
  );
}

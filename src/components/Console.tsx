import { useEffect, useRef, useState, type FormEvent, type KeyboardEvent } from "react";
import {
  AlertTriangle,
  ArrowDownToLine,
  ChevronRight,
  CornerDownLeft,
  Eraser,
  OctagonX,
  TerminalSquare,
} from "lucide-react";
import type { LogLine } from "../lib/logs";

interface Props {
  lines: LogLine[];
  live: boolean;
  onCommand: (command: string) => void;
  onClear: () => void;
}

export function Console({ lines, live, onCommand, onClear }: Props) {
  const body = useRef<HTMLDivElement>(null);
  const [follow, setFollow] = useState(true);
  const [command, setCommand] = useState("");
  const history = useRef<string[]>([]);
  const cursor = useRef(-1);

  useEffect(() => {
    if (follow && body.current) body.current.scrollTop = body.current.scrollHeight;
  }, [lines, follow]);

  const onScroll = () => {
    const el = body.current;
    if (!el) return;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
    if (atBottom !== follow) setFollow(atBottom);
  };

  const submit = (e: FormEvent) => {
    e.preventDefault();
    const value = command.trim();
    if (!value) return;
    onCommand(value);
    history.current = [value, ...history.current.filter((h) => h !== value)].slice(0, 50);
    cursor.current = -1;
    setCommand("");
    setFollow(true);
  };

  // Flechas arriba/abajo: historial de comandos, como en una terminal.
  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key !== "ArrowUp" && e.key !== "ArrowDown") return;
    e.preventDefault();
    const h = history.current;
    cursor.current =
      e.key === "ArrowUp" ? Math.min(cursor.current + 1, h.length - 1) : Math.max(cursor.current - 1, -1);
    setCommand(cursor.current === -1 ? "" : h[cursor.current]);
  };

  const warnings = lines.filter((l) => l.tone === "warn").length;
  const errors = lines.filter((l) => l.tone === "error").length;

  return (
    <section className="card console">
      <header className="card-header">
        <span className="card-title">
          <TerminalSquare size={16} />
          Consola
        </span>
        <div className="console-tools">
          {warnings > 0 && (
            <span className="console-count" data-tone="warn" title="Avisos">
              <AlertTriangle size={13} />
              {warnings}
            </span>
          )}
          {errors > 0 && (
            <span className="console-count" data-tone="error" title="Errores">
              <OctagonX size={13} />
              {errors}
            </span>
          )}
          {!follow && (
            <button className="btn btn-ghost btn-sm" onClick={() => setFollow(true)}>
              <ArrowDownToLine size={14} />
              Ir al final
            </button>
          )}
          <button className="btn btn-ghost btn-sm" onClick={onClear} disabled={!lines.length}>
            <Eraser size={14} />
            Limpiar
          </button>
        </div>
      </header>

      <div className="console-body selectable" ref={body} onScroll={onScroll}>
        {lines.length === 0 ? (
          <div className="empty console-empty">
            <span className="empty-icon">
              <TerminalSquare size={20} />
            </span>
            <strong>{live ? "Esperando mensajes del servidor…" : "La consola está vacía"}</strong>
            <span>{live ? "" : "Enciende el servidor para ver aquí lo que va ocurriendo."}</span>
          </div>
        ) : (
          lines.map((l) => (
            <div key={l.id} className="log" data-tone={l.tone} title={l.source ?? undefined}>
              <span className="log-time">{l.time ?? ""}</span>
              <span className="log-msg">
                {l.tone === "command" && <span className="log-prompt">&gt; </span>}
                {l.message}
              </span>
            </div>
          ))
        )}
      </div>

      <form className="console-input" onSubmit={submit}>
        <ChevronRight size={16} className="console-prompt" />
        <input
          value={command}
          onChange={(e) => setCommand(e.target.value)}
          onKeyDown={onKeyDown}
          placeholder={
            live
              ? "Escribe un comando: say hola, op Steve, time set day…"
              : "Enciende el servidor para escribir comandos"
          }
          disabled={!live}
          spellCheck={false}
          autoComplete="off"
          aria-label="Comando para el servidor"
        />
        <button
          className="btn btn-ghost btn-icon btn-sm"
          type="submit"
          disabled={!live || !command.trim()}
          aria-label="Enviar comando"
        >
          <CornerDownLeft size={15} />
        </button>
      </form>
    </section>
  );
}

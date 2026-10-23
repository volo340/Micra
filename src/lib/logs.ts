// Interpretación de las líneas de la consola para colorearlas.

export type LogTone = "info" | "warn" | "error" | "join" | "leave" | "ready" | "command" | "plain";

export interface LogLine {
  id: number;
  time: string | null;
  source: string | null;
  message: string;
  tone: LogTone;
}

// [12:00:00] [Server thread/INFO]: mensaje
// [12:00:00] [Server thread/INFO] [minecraft/DedicatedServer]: mensaje   (Forge/NeoForge)
const STANDARD = /^\[(\d{2}:\d{2}:\d{2})\] \[([^\]]*?)\/([A-Z]+)\](?: \[[^\]]*\])?: ?(.*)$/;
// eslint-disable-next-line no-control-regex
const ANSI = /\u001b\[[0-9;]*[A-Za-z]/g;
const SYSTEM_CHAT = "System chat: ";

let nextId = 1;

export function parseLogLine(raw: string, stream: "stdout" | "stderr"): LogLine {
  const text = raw.replace(ANSI, "");
  const m = STANDARD.exec(text);
  if (!m) {
    return { id: nextId++, time: null, source: null, message: text, tone: stream === "stderr" ? "error" : "plain" };
  }
  const [, time, source, level, message] = m;
  // Minecraft 26.3 y posteriores anteponen "System chat: " a los avisos del servidor.
  const notice = message.startsWith(SYSTEM_CHAT) ? message.slice(SYSTEM_CHAT.length) : message;
  let tone: LogTone = "info";
  if (level === "WARN") tone = "warn";
  else if (level === "ERROR" || level === "FATAL") tone = "error";
  else if (/^\w{1,16}(?: \(formerly known as \w{1,16}\))? joined the game$/.test(notice)) tone = "join";
  else if (/^\w{1,16} left the game$/.test(notice)) tone = "leave";
  else if (message.startsWith("Done (")) tone = "ready";
  return { id: nextId++, time, source, message, tone };
}

export function commandLine(command: string): LogLine {
  return { id: nextId++, time: null, source: null, message: command, tone: "command" };
}

import { useState } from "react";
import { CheckCircle2, Globe2, Settings2, Wifi, XCircle } from "lucide-react";
import { CopyButton } from "./ui";
import { useT } from "../i18n";
import { api, errorMessage } from "../lib/api";
import type { NetworkInfo, ServerSummary } from "../lib/types";

/** "dominio:puerto", o solo "dominio" cuando es el puerto de siempre. */
function splitAddress(address: string): [string, number] {
  const m = /^(.*):(\d+)$/.exec(address);
  return m ? [m[1], Number(m[2])] : [address, 25565];
}

type Note = { tone: "info" | "warning" | "critical"; text: string };

interface Props {
  server: ServerSummary;
  network: NetworkInfo | null;
  onConfigure: () => void;
}

/** Direcciones para entrar: la de casa y, si está configurada, la de internet. */
export function ConnectCard({ server, network, onConfigure }: Props) {
  const t = useT();
  const { config, live, status } = server;
  const running = status === "running";
  const [publicIp, setPublicIp] = useState<string | null>(null);
  const [check, setCheck] = useState<{ address: string; state: "checking" | "ok" | "fail"; error?: string } | null>(
    null,
  );

  const lan = `${network?.localIp ?? "localhost"}:${config.port}`;
  const wantsInternet = config.network.upnp || config.network.tunnel;
  const { tunnel, upnp } = live;

  // El túnel manda; si no hay, la IP pública con el puerto abierto por UPnP.
  let internet: { address: string; via: string } | null = null;
  let note: Note | null = null;
  let needsIp = false;
  let suggestTunnel = false;
  if (tunnel?.kind === "ready") {
    internet = { address: tunnel.address, via: t.connect.viaTunnel };
  } else if (upnp?.kind === "open") {
    const { externalIp, wan } = upnp.mapping;
    if (wan === "cgnat" || wan === "private") {
      note = { tone: "warning", text: t.connect.cgnat };
      suggestTunnel = !config.network.tunnel;
    } else if (externalIp ?? publicIp) {
      internet = { address: `${externalIp ?? publicIp}:${config.port}`, via: t.connect.viaUpnp };
    } else {
      needsIp = true;
    }
  }
  if (!internet && !note && !needsIp) {
    if (tunnel?.kind === "starting") note = { tone: "info", text: t.connect.tunnelStarting };
    else if (tunnel?.kind === "pending") note = { tone: "info", text: t.connect.tunnelPending };
    else if (tunnel?.kind === "disabled") note = { tone: "warning", text: t.connect.tunnelDisabled(tunnel.reason) };
    else if (tunnel?.kind === "failed") note = { tone: "critical", text: t.connect.tunnelFailed(tunnel.error) };
    else if (upnp?.kind === "opening") note = { tone: "info", text: t.connect.upnpOpening };
    else if (upnp?.kind === "failed") {
      note = { tone: "warning", text: t.connect.upnpFailed(upnp.error) };
      suggestTunnel = !config.network.tunnel;
    } else if (!wantsInternet) note = { tone: "info", text: t.connect.lanOnly };
    else note = { tone: "info", text: running ? t.connect.preparing : t.connect.whenRunning };
  }

  const runCheck = async (address: string) => {
    const [host, port] = splitAddress(address);
    setCheck({ address, state: "checking" });
    try {
      setCheck({ address, state: (await api.checkReachable(host, port)) ? "ok" : "fail" });
    } catch (e) {
      setCheck({ address, state: "fail", error: errorMessage(e) });
    }
  };

  const result = internet && check?.address === internet.address ? check : null;

  return (
    <section className="card connect">
      <header className="card-header">
        <span className="card-title">
          <Globe2 size={16} />
          {t.connect.title}
        </span>
      </header>
      <div className="connect-body">
        <div className="connect-row">
          <span className="connect-label">
            <Wifi size={13} />
            {t.connect.lan}
          </span>
          <div className="connect-line">
            <span className="connect-address mono selectable" title={lan}>
              {lan}
            </span>
            <CopyButton text={lan} label={t.connect.copy} />
          </div>
        </div>

        <div className="connect-row">
          <span className="connect-label">
            <Globe2 size={13} />
            {t.connect.internet}
          </span>
          {internet ? (
            <>
              <div className="connect-line">
                <span className="connect-address mono selectable" title={internet.address}>
                  {internet.address}
                </span>
                <CopyButton text={internet.address} label={t.connect.copy} />
              </div>
              <div className="connect-meta">
                <span className="subtle">{internet.via}</span>
                {result && result.state !== "checking" && (
                  <span className="reach" data-state={result.state} title={result.error}>
                    {result.state === "ok" ? <CheckCircle2 size={13} /> : <XCircle size={13} />}
                    {result.state === "ok" ? t.connect.reachable : t.connect.unreachable}
                  </span>
                )}
                {running && (
                  <button
                    className="btn btn-ghost btn-sm"
                    onClick={() => runCheck(internet.address)}
                    disabled={result?.state === "checking"}
                  >
                    {result?.state === "checking" ? t.connect.checking : t.connect.check}
                  </button>
                )}
              </div>
            </>
          ) : needsIp ? (
            <div className="connect-meta">
              <span className="subtle">{t.connect.ipUnknown}</span>
              <button
                className="btn btn-ghost btn-sm"
                onClick={() =>
                  api
                    .publicIp()
                    .then(setPublicIp)
                    .catch(() => setPublicIp(null))
                }
              >
                {t.connect.showIp}
              </button>
            </div>
          ) : (
            note && (
              <p className="connect-note" data-tone={note.tone}>
                {note.text}
              </p>
            )
          )}
          {(!wantsInternet || suggestTunnel) && (
            <div>
              <button className="btn btn-secondary btn-sm" onClick={onConfigure}>
                <Settings2 size={14} />
                {suggestTunnel ? t.connect.useTunnel : t.connect.configure}
              </button>
            </div>
          )}
        </div>
      </div>
    </section>
  );
}

import { useEffect, useState } from "react";
import { CheckCircle2, CircleSlash, Coffee, FolderOpen, HardDrive, Plug } from "lucide-react";
import { Logo } from "../components/Logo";
import { api } from "../lib/api";
import { formatGb } from "../lib/format";
import type { AppInfo, HardwareInfo, JavaRuntime } from "../lib/types";

interface Props {
  hardware: HardwareInfo | null;
  appInfo: AppInfo | null;
  onError: (e: unknown) => void;
}

export function Settings({ hardware, appInfo, onError }: Props) {
  const [runtimes, setRuntimes] = useState<JavaRuntime[] | null>(null);

  useEffect(() => {
    api.javaRuntimes().then(setRuntimes).catch(onError);
  }, [onError]);

  return (
    <div className="page settings">
      <header className="page-header">
        <div className="page-heading">
          <h1>Ajustes</h1>
          <p className="muted">Lo que Micra sabe de tu equipo y dónde guarda las cosas.</p>
        </div>
      </header>

      <div className="settings-grid">
        <div className="settings-col">
          <section className="card card-pad settings-card">
            <h2>
              <HardDrive size={17} /> Este equipo
            </h2>
            <dl className="facts">
              <div>
                <dt>Procesador</dt>
                <dd>{hardware ? `${hardware.logicalCpus} hilos` : "…"}</dd>
              </div>
              <div>
                <dt>Memoria total</dt>
                <dd>{hardware ? formatGb(hardware.totalMemoryMb) : "…"}</dd>
              </div>
              <div>
                <dt>Máximo recomendado por servidor</dt>
                <dd>{hardware ? formatGb(hardware.recommendedMaxMb) : "…"}</dd>
              </div>
            </dl>
          </section>

          <section className="card card-pad settings-card">
            <h2>
              <Plug size={17} /> Fuentes de modpacks
            </h2>
            <ul className="integrations">
              <li>
                <span>Modrinth</span>
                <span className="integration-state" data-on>
                  <CheckCircle2 size={15} /> Activa
                </span>
              </li>
              <li>
                <span>CurseForge</span>
                {appInfo?.curseforgeEnabled ? (
                  <span className="integration-state" data-on>
                    <CheckCircle2 size={15} /> Activa
                  </span>
                ) : (
                  <span className="integration-state">
                    <CircleSlash size={15} /> Necesita una clave de API
                  </span>
                )}
              </li>
            </ul>
            {!appInfo?.curseforgeEnabled && (
              <p className="field-hint">
                CurseForge exige una clave gratuita. Quien compila Micra la añade como{" "}
                <span className="mono">MICRA_CURSEFORGE_API_KEY</span>.
              </p>
            )}
          </section>
        </div>

        <div className="settings-col">
          <section className="card card-pad settings-card">
            <h2>
              <Coffee size={17} /> Java instalado por Micra
            </h2>
            {runtimes === null ? (
              <p className="muted">Cargando…</p>
            ) : runtimes.length === 0 ? (
              <p className="muted">Todavía ninguno. Se descargará automáticamente al crear tu primer servidor.</p>
            ) : (
              <ul className="runtime-list">
                {runtimes.map((rt) => (
                  <li key={rt.major}>
                    <span className="runtime-badge">Java {rt.major}</span>
                    <span className="muted mono">{rt.releaseName}</span>
                  </li>
                ))}
              </ul>
            )}
            <p className="field-hint">Están aislados en la carpeta de Micra: no cambian el Java de tu sistema.</p>
          </section>

          <section className="card card-pad settings-card">
            <h2>
              <FolderOpen size={17} /> Datos
            </h2>
            <p className="muted">Servidores, mundos, Java y descargas se guardan en:</p>
            <code className="path selectable">{appInfo?.dataDir ?? "…"}</code>
            <div>
              <button className="btn btn-secondary btn-sm" onClick={() => api.openDataFolder().catch(onError)}>
                <FolderOpen size={14} />
                Abrir carpeta
              </button>
            </div>
          </section>
        </div>

        <section className="card card-pad about">
          <Logo size={34} />
          <div className="about-text">
            <strong>Micra {appInfo?.version ?? ""}</strong>
            <span className="muted">Tu servidor de Minecraft en tu propio PC, sin complicaciones.</span>
          </div>
        </section>
      </div>
    </div>
  );
}

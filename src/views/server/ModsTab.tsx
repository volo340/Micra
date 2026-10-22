import { useCallback, useEffect, useRef, useState } from "react";
import {
  ArrowUpCircle,
  Check,
  Download,
  Package,
  PackageSearch,
  Puzzle,
  RefreshCw,
  Search,
  ShieldAlert,
  ShieldCheck,
  Trash2,
} from "lucide-react";
import { useConfirm } from "../../components/Dialog";
import { useToast } from "../../components/Toasts";
import { Banner, CardTitle, Progress } from "../../components/ui";
import { useT, type Dict } from "../../i18n";
import { api, errorMessage, onProvisionStep } from "../../lib/api";
import { formatBytes, formatCompact, loaderName } from "../../lib/format";
import type { ModInfo, ModRemoteInfo, ModrinthHit, ProvisionStep, ScanReport, ServerSummary } from "../../lib/types";
import { useTask } from "../../lib/useTask";

function ModIcon({ url, name }: { url: string | null; name: string }) {
  const [failed, setFailed] = useState(false);
  return (
    <span className="mod-icon">
      {url && !failed ? (
        <img src={url} alt="" loading="lazy" onError={() => setFailed(true)} />
      ) : (
        name.slice(0, 1).toUpperCase()
      )}
    </span>
  );
}

function stepText(step: ProvisionStep | null, t: Dict): string {
  switch (step?.step) {
    case "downloadingFiles":
      return t.mods.packSteps.files(step.done, step.total);
    case "installingJava":
    case "downloadingJava":
      return t.mods.packSteps.java;
    case "installingLoader":
      return t.mods.packSteps.loader;
    case "finalizing":
      return t.mods.packSteps.finalizing;
    default:
      return t.mods.packSteps.fetching;
  }
}

const ruleText = (rule: string, t: Dict) => (t.mods.rules as Record<string, string>)[rule] ?? rule;

interface Props {
  server: ServerSummary;
  onError: (e: unknown) => void;
  onChanged: () => void;
}

/** Pestaña Mods: lista, actualizaciones, instalar desde Modrinth y análisis de malware. */
export function ModsTab({ server, onError, onChanged }: Props) {
  const t = useT();
  const toast = useToast();
  const { config, status, live } = server;
  const id = config.id;
  const stopped = status === "stopped" || status === "crashed";
  const [mods, setMods] = useState<ModInfo[] | null>(null);
  const [remote, setRemote] = useState<Record<string, ModRemoteInfo>>({});
  const [checking, setChecking] = useState(false);
  const [working, setWorking] = useState<string | null>(null);
  const [packStep, setPackStep] = useState<ProvisionStep | null>(null);
  const [packChecked, setPackChecked] = useState(false);
  const [report, setReport] = useState<ScanReport | null>(null);
  const { progress, run } = useTask(id);
  const [dialog, confirm] = useConfirm();
  const locked = !stopped || working !== null || server.busy;

  const checkUpdates = useCallback(
    async (quiet: boolean) => {
      setChecking(true);
      try {
        const list = await api.checkModUpdates(id);
        setRemote(Object.fromEntries(list.map((r) => [r.sha1, r])));
      } catch (e) {
        if (!quiet) onError(e);
      } finally {
        setChecking(false);
      }
    },
    [id, onError],
  );

  const load = useCallback(async () => {
    try {
      const list = await api.listMods(id);
      setMods(list);
      return list;
    } catch (e) {
      setMods([]);
      onError(e);
      return [];
    }
  }, [id, onError]);

  useEffect(() => {
    load().then((list) => {
      if (list.length) checkUpdates(true);
    });
  }, [load, checkUpdates]);

  const withWork = async (key: string, action: () => Promise<void>) => {
    setWorking(key);
    try {
      await action();
    } catch (e) {
      onError(e);
    } finally {
      setWorking(null);
    }
  };

  const toggle = (m: ModInfo) =>
    withWork(m.fileName, async () => {
      await api.setModEnabled(id, m.fileName, !m.enabled);
      await load();
    });

  const remove = async (m: ModInfo) => {
    const name = remote[m.sha1]?.title ?? m.name ?? m.fileName;
    const ok = await confirm({
      title: t.mods.removeTitle(name),
      message: m.fromPack ? t.mods.removeFromPack : t.mods.removeMessage,
      confirmLabel: t.common.remove,
      danger: true,
    });
    if (ok === null) return;
    withWork(m.fileName, async () => {
      await api.removeMod(id, m.fileName);
      await load();
    });
  };

  const updateOne = (m: ModInfo, versionId: string) =>
    withWork(m.fileName, async () => {
      await api.updateMod(id, m.fileName, versionId);
      const list = await load();
      if (list.length) await checkUpdates(true);
      onChanged();
    });

  const updatable = (mods ?? []).filter((m) => remote[m.sha1]?.update);

  const updateAll = () =>
    withWork("all", async () => {
      let done = 0;
      for (const m of updatable) {
        const update = remote[m.sha1]?.update;
        if (!update) continue;
        await api.updateMod(id, m.fileName, update.versionId);
        done++;
      }
      const list = await load();
      if (list.length) await checkUpdates(true);
      onChanged();
      toast({ tone: "success", title: t.mods.updatedAll(done) });
    });

  const checkPack = () =>
    withWork("pack-check", async () => {
      const update = await api.checkPackUpdate(id);
      setPackChecked(true);
      if (!update) toast({ tone: "info", title: t.mods.packUpToDate });
    });

  const updatePack = async () => {
    const update = live.packUpdate;
    if (!update) return;
    const ok = await confirm({
      title: t.mods.packUpdateTitle(update.latest),
      message: t.mods.packUpdateMessage,
      confirmLabel: t.mods.packUpdate,
    });
    if (ok === null) return;
    const unlisten = await onProvisionStep(setPackStep);
    await withWork("pack", async () => {
      try {
        await api.updatePack(id, update.target);
        toast({ tone: "success", title: t.mods.packUpdated(update.latest) });
        onChanged();
        const list = await load();
        if (list.length) await checkUpdates(true);
      } finally {
        unlisten();
        setPackStep(null);
      }
    });
  };

  const scan = async () => {
    try {
      setReport(await run("scan", () => api.scanMods(id)));
    } catch (e) {
      onError(e);
    }
  };

  const quarantine = async (path: string) => {
    const ok = await confirm({
      title: t.mods.quarantineTitle,
      message: t.mods.quarantineMessage(path),
      confirmLabel: t.mods.quarantine,
      danger: true,
    });
    if (ok === null) return;
    withWork(path, async () => {
      await api.quarantineMod(id, path);
      setReport((r) => r && { ...r, files: r.files.filter((f) => f.path !== path) });
      await load();
    });
  };

  return (
    <div className="tab-body">
      {dialog}
      {!stopped && <Banner>{t.mods.stopToChange}</Banner>}

      {config.source && (
        <section className="card card-pad settings-card">
          <CardTitle
            icon={<Package size={17} />}
            title={t.mods.packTitle}
            hint={t.mods.packHint(config.source.kind === "modrinth" ? "Modrinth" : "CurseForge")}
          />
          {live.packUpdate ? (
            <Banner
              action={
                <button className="btn btn-primary btn-sm" onClick={updatePack} disabled={locked}>
                  <ArrowUpCircle size={14} />
                  {t.mods.packUpdate}
                </button>
              }
            >
              {t.mods.packAvailable(live.packUpdate.latest, live.packUpdate.current)}
            </Banner>
          ) : (
            <div className="button-row">
              <button className="btn btn-secondary btn-sm" onClick={checkPack} disabled={working !== null}>
                <RefreshCw size={14} />
                {working === "pack-check" ? t.mods.checking : t.mods.packCheck}
              </button>
              {packChecked && (
                <span className="inline-ok">
                  <Check size={14} />
                  {t.mods.packUpToDate}
                </span>
              )}
            </div>
          )}
          {working === "pack" && <Progress done={0} total={0} label={stepText(packStep, t)} />}
        </section>
      )}

      <section className="card list-card">
        <header className="card-header">
          <span className="card-title">
            <Puzzle size={16} />
            {t.mods.installed}
            <span className="count">{mods?.length ?? 0}</span>
          </span>
          <div className="button-row">
            <button
              className="btn btn-ghost btn-sm"
              onClick={() => checkUpdates(false)}
              disabled={checking || !mods?.length}
            >
              <RefreshCw size={14} />
              {checking ? t.mods.checking : t.mods.checkUpdates}
            </button>
            {updatable.length > 0 && (
              <button className="btn btn-primary btn-sm" onClick={updateAll} disabled={locked}>
                <ArrowUpCircle size={14} />
                {t.mods.updateAll(updatable.length)}
              </button>
            )}
          </div>
        </header>
        {mods === null ? (
          <p className="list-empty">{t.common.loading}</p>
        ) : mods.length === 0 ? (
          <div className="empty">
            <span className="empty-icon">
              <Puzzle size={20} />
            </span>
            <strong>{t.mods.empty}</strong>
            <span>{t.mods.emptyHint}</span>
          </div>
        ) : (
          <ul className="rows">
            {mods.map((m) => {
              const info = remote[m.sha1];
              const title = info?.title ?? m.name ?? m.fileName;
              const version = m.version ?? info?.versionNumber;
              return (
                <li key={m.fileName} className="row" data-off={!m.enabled || undefined}>
                  <ModIcon url={info?.iconUrl ?? null} name={title} />
                  <span className="row-main">
                    <span className="row-title">
                      {title}
                      {version && <span className="subtle"> {version}</span>}
                    </span>
                    <span className="row-tags">
                      {m.fromPack && <span className="tag">{t.mods.fromPack}</span>}
                      {!m.enabled && <span className="tag">{t.mods.disabled}</span>}
                      <span className="subtle mono ellipsis" title={m.fileName}>
                        {m.fileName}
                      </span>
                      <span className="subtle">{formatBytes(m.size)}</span>
                    </span>
                  </span>
                  <span className="row-actions">
                    {info?.update && (
                      <button
                        className="btn btn-secondary btn-sm"
                        onClick={() => updateOne(m, info.update!.versionId)}
                        disabled={locked}
                      >
                        <ArrowUpCircle size={14} />
                        {t.mods.updateTo(info.update.versionNumber)}
                      </button>
                    )}
                    <input
                      type="checkbox"
                      className="switch"
                      checked={m.enabled}
                      disabled={locked}
                      onChange={() => toggle(m)}
                      aria-label={t.mods.enabledLabel(title)}
                      title={m.enabled ? t.mods.disable : t.mods.enable}
                    />
                    <button
                      className="btn btn-ghost btn-icon btn-sm"
                      onClick={() => remove(m)}
                      disabled={locked}
                      title={t.common.remove}
                      aria-label={t.common.remove}
                    >
                      <Trash2 size={14} />
                    </button>
                  </span>
                </li>
              );
            })}
          </ul>
        )}
      </section>

      <ModSearch
        server={server}
        locked={locked}
        installedProjects={new Set((mods ?? []).flatMap((m) => remote[m.sha1]?.projectId ?? []))}
        onInstalled={() => load()}
        onError={onError}
      />

      <section className="card card-pad settings-card">
        <CardTitle
          icon={<ShieldCheck size={17} />}
          title={t.mods.scanTitle}
          hint={t.mods.scanHint}
          aside={
            <button className="btn btn-secondary btn-sm" onClick={scan} disabled={progress !== null}>
              <ShieldCheck size={14} />
              {t.mods.scanNow}
            </button>
          }
        />
        {progress && <Progress done={progress.done} total={progress.total} label={t.mods.scanning} />}
        {report && !progress && (
          <>
            {report.host.length > 0 && (
              <Banner tone="critical">{t.mods.hostFound(report.host.map((f) => f.location).join(", "))}</Banner>
            )}
            {report.files.length === 0 && report.host.length === 0 ? (
              <span className="inline-ok">
                <ShieldCheck size={15} />
                {t.mods.clean(report.scanned)}
              </span>
            ) : (
              <ul className="rows rows-flush">
                {report.files.map((f) => {
                  const critical = f.findings.some((x) => x.severity === "critical");
                  return (
                    <li key={f.path} className="row">
                      <span className="row-icon" data-tone={critical ? "critical" : "warning"}>
                        <ShieldAlert size={15} />
                      </span>
                      <span className="row-main">
                        <span className="row-title mono">{f.path}</span>
                        <span className="row-tags">
                          <span className="tag" data-tone={critical ? "critical" : "warning"}>
                            {critical ? t.mods.critical : t.mods.suspicious}
                          </span>
                          <span className="subtle">
                            {[...new Set(f.findings.map((x) => ruleText(x.rule, t)))].join(" · ")}
                          </span>
                        </span>
                      </span>
                      <span className="row-actions">
                        <button className="btn btn-danger btn-sm" onClick={() => quarantine(f.path)} disabled={locked}>
                          {t.mods.quarantine}
                        </button>
                      </span>
                    </li>
                  );
                })}
              </ul>
            )}
          </>
        )}
      </section>
    </div>
  );
}

/** Buscar e instalar mods de Modrinth compatibles con el servidor. */
function ModSearch(props: {
  server: ServerSummary;
  locked: boolean;
  /** Proyectos de Modrinth que ya están en la carpeta de mods. */
  installedProjects: Set<string>;
  onInstalled: () => void;
  onError: (e: unknown) => void;
}) {
  const t = useT();
  const toast = useToast();
  const { config } = props.server;
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<ModrinthHit[] | null>(null);
  const [total, setTotal] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const [installing, setInstalling] = useState<string | null>(null);
  const [installed, setInstalled] = useState<Set<string>>(new Set());
  const request = useRef(0);

  useEffect(() => {
    const ticket = ++request.current;
    const timer = window.setTimeout(
      () => {
        setError(null);
        api
          .searchMods(config.id, query)
          .then((r) => {
            if (ticket !== request.current) return;
            setHits(r.hits);
            setTotal(r.total_hits);
          })
          .catch((e) => ticket === request.current && setError(errorMessage(e)));
      },
      query ? 350 : 0,
    );
    return () => window.clearTimeout(timer);
  }, [config.id, query]);

  const more = () =>
    api
      .searchMods(config.id, query, hits?.length ?? 0)
      .then((r) => setHits((h) => [...(h ?? []), ...r.hits]))
      .catch((e) => setError(errorMessage(e)));

  const install = async (hit: ModrinthHit) => {
    setInstalling(hit.project_id);
    try {
      const files = await api.installMod(config.id, hit.project_id);
      setInstalled((s) => new Set(s).add(hit.project_id));
      toast({ tone: "success", title: t.mods.installedOk(hit.title), message: files.join(", ") });
      props.onInstalled();
    } catch (e) {
      props.onError(e);
    } finally {
      setInstalling(null);
    }
  };

  return (
    <section className="card card-pad settings-card">
      <CardTitle
        icon={<PackageSearch size={17} />}
        title={t.mods.addTitle}
        hint={t.mods.addHint(loaderName(config.loader.kind), config.mcVersion)}
      />
      <div className="input-with-icon">
        <Search size={16} />
        <input
          className="input"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder={t.mods.searchPlaceholder}
        />
      </div>
      {error ? (
        <Banner tone="critical">{t.mods.searchFailed(error)}</Banner>
      ) : hits === null ? (
        <p className="subtle">{t.common.loading}</p>
      ) : hits.length === 0 ? (
        <p className="subtle">{t.mods.noResults}</p>
      ) : (
        <>
          <ul className="rows rows-flush">
            {hits.map((h) => (
              <li key={h.project_id} className="row">
                <ModIcon url={h.icon_url} name={h.title} />
                <span className="row-main">
                  <span className="row-title">
                    {h.title}
                    <span className="subtle"> · {h.author}</span>
                  </span>
                  <span className="row-desc">{h.description}</span>
                </span>
                <span className="row-actions">
                  <span className="subtle downloads">
                    <Download size={12} />
                    {formatCompact(h.downloads)}
                  </span>
                  {installed.has(h.project_id) || props.installedProjects.has(h.project_id) ? (
                    <span className="inline-ok">
                      <Check size={14} />
                      {t.mods.installedTag}
                    </span>
                  ) : (
                    <button
                      className="btn btn-secondary btn-sm"
                      onClick={() => install(h)}
                      disabled={props.locked || installing !== null}
                    >
                      {installing === h.project_id ? t.mods.installing : t.mods.install}
                    </button>
                  )}
                </span>
              </li>
            ))}
          </ul>
          {hits.length < total && (
            <div>
              <button className="btn btn-ghost btn-sm" onClick={more}>
                {t.mods.more}
              </button>
            </div>
          )}
        </>
      )}
    </section>
  );
}

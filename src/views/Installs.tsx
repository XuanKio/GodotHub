import { useEffect, useMemo, useState } from "react";
import {
  ArrowDownToLine, ArrowUpCircle, Box, Check, ChevronDown, Download, Ellipsis, ExternalLink,
  FileText, FolderOpen, HardDrive, Import, Info, LoaderCircle, PackageOpen,
  Play, Plus, RefreshCw, Search, Star, Trash2,
} from "lucide-react";
import {
  api, architectureLabels, channelLabels, editionLabels, errorMessage, formatDate, formatSize, isActive,
} from "../api";
import type { Architecture, Build, Channel, Edition, InstalledEngine, Job, Release } from "../api";
import { ErrorNotice, LoadingState, Modal, StatePanel } from "../components";
import type { Hub } from "../useHub";
import { updateKey } from "../updates";
import type { EngineUpdate } from "../updates";

export type InstallsTab = "installed" | "browse" | "updates";

const channels: Array<Channel | "all"> = ["stable", "beta", "rc", "dev", "all"];
const editions: Edition[] = ["standard", "dotnet"];
const architectures: Architecture[] = ["x64", "arm64", "x86"];

function installJobKey(tag: string, build: Build) {
  return `install-${tag}-${build.edition}-${build.arch}`;
}

function changelogs(release: Release) {
  const { notes, tag } = release;
  const version = tag.replace(/-stable$/, "");
  return [
    { label: "Complete changelog", curated: false, url: `https://godotengine.github.io/godot-interactive-changelog/#${version}` },
    { label: "Curated changelog", curated: true, url: `https://github.com/godotengine/godot/blob/${tag}/CHANGELOG.md` },
  ].filter(({ url }) => notes.includes(`(${url})`));
}

function ReleaseChannel({ channel }: { channel: Channel }) {
  return <span className={`pill channel-${channel}`}>{channelLabels[channel]}</span>;
}

function EditionIcon({ edition }: { edition: Edition }) {
  return (
    <span className={`version-icon version-${edition}`}>
      {edition === "dotnet" ? <span aria-hidden="true">.NET</span> : <Box size={22} aria-hidden="true" />}
    </span>
  );
}

export function Installs({
  hub,
  tab,
  setTab,
}: {
  hub: Hub;
  tab: InstallsTab;
  setTab: (tab: InstallsTab) => void;
}) {
  const [channel, setChannel] = useState<Channel | "all">("stable");
  const [edition, setEdition] = useState<Edition>("standard");
  const [selectedArchitecture, setSelectedArchitecture] = useState<Architecture | null>(null);
  const [query, setQuery] = useState("");
  const [busy, setBusy] = useState("");
  const [dialogError, setDialogError] = useState("");
  const [managedEngineId, setManagedEngineId] = useState<string | null>(null);
  const [confirmRemove, setConfirmRemove] = useState(false);
  const [releaseDetails, setReleaseDetails] = useState<{ release: Release; update?: EngineUpdate } | null>(null);
  const [jobIds, setJobIds] = useState<Record<string, string>>({});
  const architecture = selectedArchitecture ?? hub.architecture ?? "x64";
  const managedEngine = hub.installed.data.find((engine) => engine.id === managedEngineId);

  const { catalogue, loadCatalogue } = hub;
  const { data: catalogueData } = catalogue;

  useEffect(() => {
    if (tab === "browse" && !catalogueData) void loadCatalogue();
  }, [tab, catalogueData, loadCatalogue]);

  function isJobActive(key: string) {
    return hub.jobs.some((job) => job.id === jobIds[key] && isActive(job));
  }

  async function perform(key: string, action: () => Promise<void>, inDialog = false) {
    setBusy(key);
    setDialogError("");
    try {
      await action();
    } catch (error) {
      const message = errorMessage(error);
      if (inDialog) setDialogError(message);
      else hub.setNotice({ type: "error", message });
    } finally {
      setBusy("");
    }
  }

  function queueJob(key: string, job: Job) {
    hub.recordJob(job);
    setJobIds((current) => ({ ...current, [key]: job.id }));
    hub.setNotice({ type: "success", message: "Download queued.", action: "downloads" });
  }

  function importEngine() {
    void perform("import", async () => {
      const executable = await api.pickEngine();
      if (!executable) return;
      const engine = await api.importEngine(executable);
      await hub.loadInstalled(false);
      setTab("installed");
      hub.setNotice({ type: "success", message: `Godot ${engine.tag} imported.` });
    });
  }

  function install(tag: string, build: Build, inDialog = false) {
    const key = installJobKey(tag, build);
    void perform(key, async () => {
      queueJob(key, await api.install(tag, build.edition, build.arch));
      if (inDialog) setReleaseDetails(null);
    }, inDialog);
  }

  function closeEngineDialog() {
    setManagedEngineId(null);
    setConfirmRemove(false);
    setDialogError("");
  }

  function remove(engine: InstalledEngine) {
    void perform("remove", async () => {
      await api.remove(engine.id);
      closeEngineDialog();
      await hub.loadInstalled(false);
      hub.setNotice({ type: "success", message: `Godot ${engine.tag} removed from Installs.` });
    }, true);
  }

  const search = query.trim().toLowerCase();
  const { updates, updateCheck } = hub;
  const updateResultStale = Boolean(updateCheck.error || updateCheck.data?.stale);
  const selectedRelease = releaseDetails?.release;
  const selectedUpdate = releaseDetails?.update;
  const selectedChangelogs = selectedRelease ? changelogs(selectedRelease) : [];

  function openChangelog(release: Release, curated: boolean) {
    void perform("open-changelog", () => api.openChangelog(release.tag, curated), true);
  }
  const matchingReleases = useMemo(() => catalogueData?.releases.flatMap((release) => {
    if (channel !== "all" && release.channel !== channel) return [];
    if (search && !release.tag.toLowerCase().includes(search)) return [];
    const build = release.builds.find((item) => item.edition === edition && item.arch === architecture);
    return build ? [{ release, build }] : [];
  }) ?? [], [catalogueData, channel, search, edition, architecture]);

  return (
    <>
      <header className="page-heading">
        <div>
          <h1>Installs</h1>
          <p>Install and manage Godot Engine versions.</p>
        </div>
        <div className="heading-actions">
          <button type="button" className="button secondary" onClick={importEngine} disabled={Boolean(busy)}>
            {busy === "import" ? <LoaderCircle className="spin" size={17} /> : <Import size={17} />}
            Import engine
          </button>
          {tab === "installed" && (
            <button type="button" className="button primary" onClick={() => setTab("browse")}>
              <Plus size={18} />
              Install Godot
            </button>
          )}
        </div>
      </header>

      <div className="tabs" role="group" aria-label="Installs view">
        <button type="button" className={tab === "installed" ? "selected" : ""} aria-pressed={tab === "installed"} onClick={() => setTab("installed")}>
          Installed
          <span className="tab-count">{hub.installed.data.length}</span>
        </button>
        <button type="button" className={tab === "browse" ? "selected" : ""} aria-pressed={tab === "browse"} onClick={() => setTab("browse")}>
          Available releases
        </button>
        <button type="button" className={tab === "updates" ? "selected" : ""} aria-pressed={tab === "updates"} onClick={() => setTab("updates")}>
          Updates
          {updates.length > 0 && <span className="tab-count">{updates.length}</span>}
        </button>
      </div>

      {tab === "installed" ? (
        <section className="view-section" aria-label="Installed engines">
          {hub.installed.error && <ErrorNotice message={hub.installed.error} retry={() => void hub.loadInstalled()} />}
          {updates.length > 0 && !updateResultStale && !updateCheck.loading && (
            <div className="notice information-notice" role="status">
              <ArrowUpCircle size={19} aria-hidden="true" />
              <p>{updates.length === 1 ? `Godot ${updates[0].release.tag} is available for ${editionLabels[updates[0].build.edition]}.` : `${updates.length} Godot updates are available.`}</p>
              <button type="button" className="button secondary small" onClick={() => setTab("updates")}>
                View updates
              </button>
            </div>
          )}
          {hub.installed.loading && !hub.installed.data.length && <LoadingState title="Loading installed engines" />}
          {!hub.installed.loading && !hub.installed.error && !hub.installed.data.length && (
            <div className="install-empty">
              <div className="empty-art" aria-hidden="true">
                <span className="empty-orbit" />
                <span className="empty-orbit orbit-wide" />
                <span className="hero-icon"><Box size={34} strokeWidth={1.5} /></span>
              </div>
              <h2>No engines installed</h2>
              <p>Install a release from GitHub or import a Godot editor already on this PC.</p>
              <button type="button" className="button primary" onClick={() => setTab("browse")}>
                <ArrowDownToLine size={17} />
                Browse releases
              </button>
            </div>
          )}
          {hub.installed.data.length > 0 && (
            <ul className="item-list">
              {hub.installed.data.map((engine) => (
                <li className="item-row" key={engine.id}>
                  <EditionIcon edition={engine.edition} />
                  <div className="item-content">
                    <div className="item-title">
                      <h2>Godot {engine.tag}</h2>
                      {engine.isDefault && <span className="pill default-pill"><Star size={12} />Default</span>}
                      {!engine.managed && <span className="pill">Imported</span>}
                    </div>
                    <p className="item-meta">
                      <span>{editionLabels[engine.edition]}</span>
                      <span>{architectureLabels[engine.arch] ?? engine.arch}</span>
                      {engine.templatesInstalled && <span>Export templates installed</span>}
                    </p>
                    <p className="item-path" title={engine.executable}>{engine.executable}</p>
                  </div>
                  <div className="row-actions">
                    <button
                      type="button"
                      className="button secondary"
                      disabled={Boolean(busy)}
                      aria-label={`Open Godot ${engine.tag}`}
                      onClick={() => void perform(`launch-${engine.id}`, async () => {
                        await api.launch(engine.id);
                        hub.setNotice({ type: "success", message: `Godot ${engine.tag} launched.` });
                      })}
                    >
                      {busy === `launch-${engine.id}` ? <LoaderCircle className="spin" size={16} /> : <Play size={16} />}
                      Open
                    </button>
                    <button
                      type="button"
                      className="button secondary icon-button"
                      title="Manage engine"
                      aria-label={`Manage Godot ${engine.tag}`}
                      disabled={Boolean(busy)}
                      onClick={() => setManagedEngineId(engine.id)}
                    >
                      <Ellipsis size={20} />
                    </button>
                  </div>
                </li>
              ))}
            </ul>
          )}
        </section>
      ) : tab === "updates" ? (
        <section className="view-section" aria-label="Engine updates">
          <div className="update-toolbar">
            <p>Install updates alongside existing engines. Project files and engine assignments stay in place.</p>
            <button type="button" className="button secondary" disabled={updateCheck.loading} onClick={() => void hub.checkUpdates()}>
              <RefreshCw size={17} className={updateCheck.loading ? "spin" : undefined} />
              {updateCheck.loading ? "Checking…" : "Check now"}
            </button>
          </div>
          {hub.installed.error && <ErrorNotice message={hub.installed.error} retry={() => void hub.loadInstalled()} />}
          {updateCheck.error && <ErrorNotice message={updateCheck.error} />}
          {updateCheck.data?.stale && !updateCheck.error && (
            <div className="notice information-notice" role="status">
              <Info size={18} aria-hidden="true" />
              <p>{updateCheck.data.warning || "Showing cached releases. Check again to reach GitHub."}</p>
            </div>
          )}
          {(updateCheck.loading || hub.installed.loading) && updates.length === 0 && <LoadingState title="Checking for engine updates" />}
          {!updateCheck.loading && !hub.installed.loading && !hub.installed.error && updates.length === 0 && (
            !hub.installed.data.length ? (
              <StatePanel icon={Box} title="No engines to check" description="Install or import a Godot editor to see compatible updates.">
                <button type="button" className="button secondary" onClick={() => setTab("installed")}>View installs</button>
              </StatePanel>
            ) : updateCheck.data && !updateResultStale ? (
              <StatePanel icon={Check} title="Installed engines are up to date" description="GitHub has no newer matching release for your installed engines." />
            ) : !updateResultStale && (
              <StatePanel icon={ArrowUpCircle} title="Check for engine updates" description="Check GitHub to find newer releases for the editions and architectures you have installed." />
            )
          )}
          {updates.length > 0 && (
            <ul className="item-list">
              {updates.map((update) => {
                const { current, release, build } = update;
                const key = installJobKey(release.tag, build);
                const installing = busy === key || isJobActive(key);
                return (
                  <li key={updateKey(update)} className="item-row release-row">
                    <EditionIcon edition={build.edition} />
                    <div className="item-content">
                      <div className="item-title"><h2>Godot {release.tag}</h2><ReleaseChannel channel={release.channel} /></div>
                      <p className="item-meta">
                        <span>From {current.tag}</span><span>{editionLabels[build.edition]}</span>
                        <span>{architectureLabels[build.arch]}</span><span>{formatSize(build.engine.size)}</span>
                      </p>
                    </div>
                    <div className="row-actions">
                      <button type="button" className="button secondary" onClick={() => {
                        setDialogError("");
                        setReleaseDetails({ release, update });
                      }}><FileText size={16} /> Changelog</button>
                      <button type="button" className="button primary" disabled={Boolean(busy) || installing} onClick={() => install(release.tag, build)}>
                        {installing ? <LoaderCircle className="spin" size={16} /> : <Download size={16} />}
                        {installing ? "Queued" : "Install alongside"}
                      </button>
                    </div>
                  </li>
                );
              })}
            </ul>
          )}
        </section>
      ) : (
        <section className="view-section" aria-label="Available releases">
          <div className="release-toolbar">
            <label className="search-field">
              <Search size={17} aria-hidden="true" />
              <input type="search" value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Find a version" aria-label="Find a version" />
            </label>
            <label className="select-field" title="Release channel">
              <select aria-label="Release channel" value={channel} onChange={(event) => setChannel(event.target.value as Channel | "all")}>
                {channels.map((item) => <option key={item} value={item}>{item === "all" ? "All channels" : channelLabels[item]}</option>)}
              </select>
              <ChevronDown size={14} aria-hidden="true" />
            </label>
            <label className="select-field" title="Engine edition">
              <select aria-label="Engine edition" value={edition} onChange={(event) => setEdition(event.target.value as Edition)}>
                {editions.map((item) => <option key={item} value={item}>{editionLabels[item]}</option>)}
              </select>
              <ChevronDown size={14} aria-hidden="true" />
            </label>
            <label className="select-field" title="Build architecture">
              <select aria-label="Build architecture" value={architecture} onChange={(event) => setSelectedArchitecture(event.target.value as Architecture)}>
                {architectures.map((item) => <option key={item} value={item}>{architectureLabels[item]}</option>)}
              </select>
              <ChevronDown size={14} aria-hidden="true" />
            </label>
            <button
              type="button"
              title="Refresh releases"
              aria-label="Refresh releases"
              className="button secondary icon-button"
              disabled={catalogue.loading}
              onClick={() => void loadCatalogue(true)}
            >
              <RefreshCw size={17} className={catalogue.loading ? "spin" : undefined} />
            </button>
          </div>

          {catalogue.error && <ErrorNotice message={catalogue.error} retry={() => void loadCatalogue(true)} />}
          {catalogueData?.stale && !catalogue.error && (
            <div className="notice information-notice" role="status">
              <Info size={18} aria-hidden="true" />
              <p>{catalogueData.warning || "Showing the last available release list. Refresh to try GitHub again."}</p>
            </div>
          )}
          {catalogue.loading && !catalogueData && <LoadingState title="Fetching releases from GitHub" />}
          {!catalogue.loading && !catalogue.error && catalogueData && matchingReleases.length === 0 && (
            <StatePanel icon={PackageOpen} title="No matching releases" description="Change the filters to find other Windows builds.">
              <button type="button" className="button secondary" onClick={() => { setChannel("all"); setQuery(""); setSelectedArchitecture(null); setEdition("standard"); }}>
                Reset filters
              </button>
            </StatePanel>
          )}
          {matchingReleases.length > 0 && (
            <ul className="item-list">
              {matchingReleases.map(({ release, build }) => {
                const key = installJobKey(release.tag, build);
                const alreadyInstalled = hub.installed.data.some((engine) => (
                  engine.tag === release.tag && engine.edition === edition && engine.arch === architecture
                ));
                const downloading = isJobActive(key) || busy === key;
                return (
                  <li className="item-row release-row" key={key}>
                    <EditionIcon edition={build.edition} />
                    <div className="item-content">
                      <div className="item-title">
                        <h2>Godot {release.tag}</h2>
                        <ReleaseChannel channel={release.channel} />
                      </div>
                      <p className="item-meta">
                        <span>{editionLabels[build.edition]}</span>
                        <span>{architectureLabels[build.arch]}</span>
                        {build.engine.size > 0 && <span>{formatSize(build.engine.size)}</span>}
                        {formatDate(release.publishedAt) && <span>{formatDate(release.publishedAt)}</span>}
                      </p>
                    </div>
                    <div className="row-actions">
                      <button type="button" className="button ghost icon-button" aria-label={`Release notes for ${release.tag}`} title="Release notes" onClick={() => {
                        setReleaseDetails({ release });
                        setDialogError("");
                      }}>
                        <FileText size={18} />
                      </button>
                      <button
                        type="button"
                        className={`button ${alreadyInstalled ? "ghost" : "secondary"} release-install`}
                        disabled={Boolean(busy) || alreadyInstalled || downloading}
                        aria-label={`Install Godot ${release.tag} ${editionLabels[build.edition]} ${architectureLabels[build.arch]}`}
                        onClick={() => install(release.tag, build)}
                      >
                        {alreadyInstalled ? <Check size={16} /> : downloading ? <LoaderCircle className="spin" size={16} /> : <Download size={16} />}
                        {alreadyInstalled ? "Installed" : downloading ? "Queued" : "Install"}
                      </button>
                    </div>
                  </li>
                );
              })}
            </ul>
          )}
          {catalogueData && <p className="source-note"><Info size={14} aria-hidden="true" /> Release source · godotengine/godot-builds</p>}
        </section>
      )}

      {managedEngine && (
        <Modal title={confirmRemove ? `Remove Godot ${managedEngine.tag}?` : `Godot ${managedEngine.tag}`} onClose={closeEngineDialog} busy={Boolean(busy)}>
          {dialogError && <ErrorNotice message={dialogError} />}
          {confirmRemove ? (
            <>
              <p className="modal-copy">
                {managedEngine.managed ? "This deletes the editor files installed by Godot Hub for this version." : "This removes the imported engine registration from Godot Hub."}
              </p>
              <div className="modal-actions">
                <button type="button" className="button secondary" disabled={Boolean(busy)} onClick={() => { setConfirmRemove(false); setDialogError(""); }}>
                  Cancel
                </button>
                <button type="button" className="button danger" disabled={Boolean(busy)} onClick={() => remove(managedEngine)}>
                  {busy === "remove" ? <LoaderCircle size={16} className="spin" /> : <Trash2 size={16} />}
                  {managedEngine.managed ? "Remove engine" : "Remove registration"}
                </button>
              </div>
            </>
          ) : (
            <>
              <p className="setting-path" title={managedEngine.executable}>{managedEngine.executable}</p>
              <div className="dialog-action-list">
                <button type="button" className="button secondary" disabled={Boolean(busy) || managedEngine.isDefault} onClick={() => void perform("default", async () => {
                  await api.setDefault(managedEngine.id);
                  await hub.loadInstalled(false);
                }, true)}>
                  <Star size={17} /> {managedEngine.isDefault ? "Default version" : "Set as default"}
                </button>
                <button type="button" className="button secondary" disabled={Boolean(busy) || managedEngine.templatesInstalled || isJobActive(`templates-${managedEngine.id}`)} onClick={() => {
                  const key = `templates-${managedEngine.id}`;
                  void perform(key, async () => {
                    queueJob(key, await api.templates(managedEngine.id));
                    closeEngineDialog();
                  }, true);
                }}>
                  <HardDrive size={17} />
                  {managedEngine.templatesInstalled ? "Export templates installed" : isJobActive(`templates-${managedEngine.id}`) ? "Export templates queued" : "Install export templates"}
                </button>
                <button type="button" className="button secondary" disabled={Boolean(busy)} onClick={() => void perform("reveal", async () => {
                  await api.reveal(managedEngine.id);
                }, true)}>
                  <FolderOpen size={17} /> Show in File Explorer
                </button>
                <button type="button" className="button secondary" disabled={Boolean(busy)} onClick={() => void perform("shortcut", async () => {
                  await api.shortcut(managedEngine.id);
                  closeEngineDialog();
                  hub.setNotice({ type: "success", message: "Desktop shortcut created." });
                }, true)}>
                  <ExternalLink size={17} /> Create desktop shortcut
                </button>
                <button type="button" className="button secondary danger-outline" disabled={Boolean(busy)} onClick={() => { setDialogError(""); setConfirmRemove(true); }}>
                  <Trash2 size={17} /> {managedEngine.managed ? "Remove engine" : "Remove registration"}
                </button>
              </div>
            </>
          )}
        </Modal>
      )}

      {selectedRelease && (
        <Modal title={`Godot ${selectedRelease.tag}`} busy={Boolean(busy)} onClose={() => {
          setReleaseDetails(null);
          setDialogError("");
        }}>
          <div className="release-details-heading">
            <ReleaseChannel channel={selectedRelease.channel} />
            <time dateTime={selectedRelease.publishedAt}>{formatDate(selectedRelease.publishedAt)}</time>
          </div>
          <h3>{selectedUpdate ? "Changelog" : "Release notes"}</h3>
          {selectedChangelogs.length > 0 && (
            <div className="changelog-links">
              {selectedChangelogs.map(({ label, curated }) => (
                <button key={label} type="button" className="text-button" disabled={Boolean(busy)} onClick={() => openChangelog(selectedRelease, curated)}>
                  {label} <ExternalLink size={14} />
                </button>
              ))}
            </div>
          )}
          <p className="release-notes">{selectedRelease.notes.trim() || "This GitHub release has no notes."}</p>
          {dialogError && <ErrorNotice message={dialogError} />}
          <div className="modal-actions">
            <button type="button" className="button secondary" disabled={Boolean(busy)} onClick={() => void perform("open-release", async () => {
              await api.openRelease(selectedRelease.tag);
            }, true)}>
              <ExternalLink size={16} /> Open on GitHub
            </button>
            {selectedUpdate && updates.some((update) => updateKey(update) === updateKey(selectedUpdate)) && (
              <button type="button" className="button primary" disabled={Boolean(busy) || isJobActive(installJobKey(selectedRelease.tag, selectedUpdate.build))}
                onClick={() => install(selectedRelease.tag, selectedUpdate.build, true)}>
                <Download size={16} /> Install alongside
              </button>
            )}
          </div>
        </Modal>
      )}
    </>
  );
}

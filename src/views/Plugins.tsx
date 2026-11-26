import { useCallback, useEffect, useRef, useState } from "react";
import {
  Archive, ArrowLeft, ArrowRight, ChevronDown, Download, FolderKanban, Info, LoaderCircle,
  PackageOpen, Puzzle, RefreshCw, Search, Trash2, X,
} from "lucide-react";
import { api, errorMessage } from "../api";
import type { AssetSearch, LibraryPlugin, Project, ProjectPlugin } from "../api";
import { ErrorNotice, LoadingState, Modal, StatePanel } from "../components";
import type { Hub } from "../useHub";

type PluginTab = "installed" | "library";

function AssetLibrary({
  projectId,
  busy,
  onInstall,
}: {
  projectId: string;
  busy: string;
  onInstall: (asset: LibraryPlugin) => void;
}) {
  const [input, setInput] = useState("");
  const [request, setRequest] = useState({ query: "", page: 0 });
  const [results, setResults] = useState<AssetSearch | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");

  useEffect(() => {
    let disposed = false;
    setLoading(true);
    setError("");
    setResults(null);
    void api.searchPlugins(projectId, request.query, request.page)
      .then((data) => { if (!disposed) setResults(data); })
      .catch((cause: unknown) => { if (!disposed) setError(errorMessage(cause)); })
      .finally(() => { if (!disposed) setLoading(false); });
    return () => { disposed = true; };
  }, [projectId, request]);

  return (
    <div className="plugin-library">
      <form
        className="plugin-search"
        aria-label="Search Godot Asset Library"
        onSubmit={(event) => { event.preventDefault(); setRequest({ query: input.trim(), page: 0 }); }}
      >
        <label className="search-field">
          <Search size={17} aria-hidden="true" />
          <input type="search" value={input} onChange={(event) => setInput(event.target.value)} placeholder="Search Godot Asset Library" aria-label="Search Godot Asset Library" disabled={Boolean(busy)} />
        </label>
        <button type="submit" className="button secondary" disabled={Boolean(busy)}>Search</button>
      </form>
      {error && <ErrorNotice message={error} retry={() => setRequest((current) => ({ ...current }))} />}
      {loading && <LoadingState title="Loading Asset Library" />}
      {!loading && results && !results.assets.length && (
        <StatePanel icon={Search} title="No matching addons" description="Try another search term." />
      )}
      {results && results.assets.length > 0 && (
        <>
          <ul className="item-list">
            {results.assets.map((asset) => (
              <li className="item-row plugin-row" key={asset.id}>
                <span className="version-icon"><Puzzle size={21} aria-hidden="true" /></span>
                <div className="item-content">
                  <div className="item-title"><h3>{asset.title}</h3></div>
                  <p className="item-meta">
                    {asset.author && <span>{asset.author}</span>}
                    {asset.version && <span>{asset.version}</span>}
                    {asset.category && <span>{asset.category}</span>}
                    {asset.license && <span>{asset.license}</span>}
                  </p>
                </div>
                <button type="button" className="button secondary" aria-label={`Install ${asset.title}`} onClick={() => onInstall(asset)} disabled={Boolean(busy)}>
                  {busy === `asset-${asset.id}` ? <LoaderCircle size={16} className="spin" /> : <Download size={16} />}
                  {busy === `asset-${asset.id}` ? "Installing…" : "Install"}
                </button>
              </li>
            ))}
          </ul>
          {results.pageCount > 1 && (
            <nav className="plugin-pagination" aria-label="Asset Library pages">
              <button type="button" className="button secondary small" disabled={results.page <= 0 || Boolean(busy)} onClick={() => setRequest((current) => ({ ...current, page: results.page - 1 }))}>
                <ArrowLeft size={15} aria-hidden="true" /> Previous
              </button>
              <span>Page {results.page + 1} of {results.pageCount}</span>
              <button type="button" className="button secondary small" disabled={results.page + 1 >= results.pageCount || Boolean(busy)} onClick={() => setRequest((current) => ({ ...current, page: results.page + 1 }))}>
                Next <ArrowRight size={15} aria-hidden="true" />
              </button>
            </nav>
          )}
        </>
      )}
    </div>
  );
}

function ProjectPlugins({
  project,
  busy,
  setBusy,
  onNotice,
  canBrowseLibrary,
  onProjects,
}: {
  project: Project;
  busy: string;
  setBusy: (value: string) => void;
  onNotice: (message: string) => void;
  canBrowseLibrary: boolean;
  onProjects: () => void;
}) {
  const [tab, setTab] = useState<PluginTab>("installed");
  const [plugins, setPlugins] = useState<ProjectPlugin[]>([]);
  const [loading, setLoading] = useState(true);
  const [listError, setListError] = useState("");
  const [actionError, setActionError] = useState("");
  const [backup, setBackup] = useState("");
  const [removing, setRemoving] = useState<ProjectPlugin | null>(null);
  const loadSequence = useRef(0);

  const loadPlugins = useCallback(async () => {
    const sequence = ++loadSequence.current;
    setLoading(true);
    setListError("");
    try {
      const installed = await api.projectPlugins(project.id);
      if (loadSequence.current === sequence) setPlugins(installed);
    } catch (cause) {
      if (loadSequence.current === sequence) setListError(errorMessage(cause));
    } finally {
      if (loadSequence.current === sequence) setLoading(false);
    }
  }, [project.id]);

  useEffect(() => { void loadPlugins(); }, [loadPlugins]);

  async function perform(action: string, callback: () => Promise<void>) {
    if (busy) return;
    setBusy(action);
    setActionError("");
    try {
      await callback();
    } catch (cause) {
      setActionError(errorMessage(cause));
    } finally {
      setBusy("");
    }
  }

  async function showInstalled(added: ProjectPlugin[]) {
    setTab("installed");
    await loadPlugins();
    const label = added.length === 1 ? added[0].name : `${added.length} addons`;
    onNotice(`${label} installed in ${project.name}.`);
  }

  function importZip() {
    void perform("archive", async () => {
      const archivePath = await api.pickPluginArchive();
      if (!archivePath) return;
      setBusy("archive-install");
      await showInstalled(await api.installProjectPlugins(project.id, archivePath));
    });
  }

  function installAsset(asset: LibraryPlugin) {
    void perform(`asset-${asset.id}`, async () => {
      await showInstalled(await api.installLibraryPlugin(project.id, asset.id));
    });
  }

  function removePlugin(plugin: ProjectPlugin) {
    void perform("remove", async () => {
      const path = await api.removeProjectPlugin(project.id, plugin.id);
      setBackup(path);
      setRemoving(null);
      await loadPlugins();
      onNotice(`${plugin.name} removed from ${project.name}.`);
    });
  }

  return (
    <>
      <div className="plugin-tab-bar">
        <div className="tabs" role="group" aria-label="Plugins view">
          <button type="button" className={tab === "installed" ? "selected" : ""} aria-pressed={tab === "installed"} disabled={Boolean(busy)} onClick={() => { setTab("installed"); setActionError(""); }}>
            Installed <span className="tab-count">{plugins.length}</span>
          </button>
          <button type="button" className={tab === "library" ? "selected" : ""} aria-pressed={tab === "library"} disabled={Boolean(busy)} onClick={() => { setTab("library"); setActionError(""); }}>
            Asset Library
          </button>
        </div>
        <button type="button" className="button primary" onClick={importZip} disabled={Boolean(busy) || loading}>
          {busy.startsWith("archive") ? <LoaderCircle size={17} className="spin" /> : <Archive size={17} />}
          {busy === "archive-install" ? "Importing…" : "Import ZIP"}
        </button>
      </div>
      <div className="view-section">
        {(busy === "archive-install" || busy.startsWith("asset-")) && <p className="plugin-working" role="status">Installing addons in {project.name}…</p>}
        {actionError && !removing && <ErrorNotice message={actionError} />}
        {tab === "installed" ? (
          <>
            {listError && <ErrorNotice message={listError} retry={() => void loadPlugins()} />}
            {backup && (
              <div className="notice information-notice plugin-backup" role="status">
                <Info size={18} aria-hidden="true" />
                <p>Backup saved to <span>{backup}</span></p>
                <button type="button" className="button icon-button ghost small" aria-label="Dismiss backup location" onClick={() => setBackup("")}><X size={16} /></button>
              </div>
            )}
            {plugins.length > 0 && (
              <div className="notice information-notice">
                <Info size={18} aria-hidden="true" />
                <p>Enable addons in Godot Editor under <strong>Project Settings → Plugins</strong>.</p>
                <button type="button" className="button icon-button ghost small" aria-label="Refresh installed addons" title="Refresh installed addons" onClick={() => void loadPlugins()} disabled={loading || Boolean(busy)}>
                  <RefreshCw size={16} className={loading ? "spin" : undefined} />
                </button>
              </div>
            )}
            {loading && !plugins.length && <LoadingState title="Loading installed addons" />}
            {!loading && !listError && !plugins.length && (
              <StatePanel icon={PackageOpen} title="No addons installed" description="Import a ZIP or browse the Godot Asset Library." />
            )}
            {plugins.length > 0 && (
              <ul className="item-list">
                {plugins.map((plugin) => (
                  <li className="item-row plugin-row" key={plugin.id}>
                    <span className="version-icon"><Puzzle size={21} aria-hidden="true" /></span>
                    <div className="item-content">
                      <div className="item-title">
                        <h3>{plugin.name}</h3>
                        <span className={`pill${plugin.managed ? " plugin-managed" : ""}`}>{plugin.managed ? "Hub-managed" : "Project addon"}</span>
                      </div>
                      {(plugin.author || plugin.version) && (
                        <p className="item-meta">
                          {plugin.author && <span>{plugin.author}</span>}
                          {plugin.version && <span>{plugin.version}</span>}
                        </p>
                      )}
                      {plugin.description && <p className="plugin-description">{plugin.description}</p>}
                      <p className="item-path" title={plugin.directory}>{plugin.directory}</p>
                    </div>
                    {plugin.managed && (
                      <button type="button" className="button secondary" aria-label={`Remove ${plugin.name}`} disabled={loading || Boolean(busy)} onClick={() => { setActionError(""); setRemoving(plugin); }}>
                        <Trash2 size={16} /> Remove
                      </button>
                    )}
                  </li>
                ))}
              </ul>
            )}
          </>
        ) : canBrowseLibrary ? (
          <AssetLibrary projectId={project.id} busy={busy} onInstall={installAsset} />
        ) : (
          <StatePanel icon={Puzzle} title="Choose a Godot editor" description="Assign an editor to find Asset Library addons for this project's version.">
            <button type="button" className="button secondary" onClick={onProjects}>Go to Projects</button>
          </StatePanel>
        )}
      </div>
      {removing && (
        <Modal title={`Remove ${removing.name}?`} onClose={() => { setRemoving(null); setActionError(""); }} busy={busy === "remove"}>
          {actionError && <ErrorNotice message={actionError} />}
          <p className="modal-copy">Godot Hub will move this addon out of {project.name} and keep a backup.</p>
          <p className="setting-path plugin-remove-path">{removing.directory}</p>
          <div className="modal-actions">
            <button type="button" className="button secondary" onClick={() => { setRemoving(null); setActionError(""); }} disabled={Boolean(busy)}>Cancel</button>
            <button type="button" className="button danger" onClick={() => removePlugin(removing)} disabled={Boolean(busy)}>
              {busy === "remove" ? <LoaderCircle size={16} className="spin" /> : <Trash2 size={16} />} Remove addon
            </button>
          </div>
        </Modal>
      )}
    </>
  );
}

export function Plugins({ hub, onProjects }: { hub: Hub; onProjects: () => void }) {
  const [projects, setProjects] = useState<Project[]>([]);
  const [selectedId, setSelectedId] = useState("");
  const [busy, setBusy] = useState("");
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const project = projects.find((item) => item.id === selectedId) ?? projects.find((item) => !item.missing) ?? projects[0];

  const loadProjects = useCallback(async () => {
    setLoading(true);
    setError("");
    try {
      setProjects(await api.projects());
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { void loadProjects(); }, [loadProjects]);

  return (
    <div className="plugin-view">
      <header className="page-heading">
        <div>
          <h1>Plugins</h1>
          <p>Manage editor addons for a Godot project.</p>
        </div>
      </header>
      {error && <ErrorNotice message={error} retry={() => void loadProjects()} />}
      {loading && !projects.length && <LoadingState title="Loading projects" />}
      {!loading && !error && !project && (
        <StatePanel icon={FolderKanban} title="No project to manage" description="Create or import a project to install addons.">
          <button type="button" className="button primary" onClick={onProjects}><FolderKanban size={17} /> Go to Projects</button>
        </StatePanel>
      )}
      {project && (
        <>
          <div className="plugin-context">
            <label className="plugin-project-picker">
              <span>Project</span>
              <span className="select-field">
                <select value={project.id} onChange={(event) => setSelectedId(event.target.value)} disabled={Boolean(busy) || loading} aria-describedby="plugin-project-path">
                  {projects.map((item) => <option key={item.id} value={item.id}>{item.name}{item.missing ? " (missing)" : ""}</option>)}
                </select>
                <ChevronDown size={14} aria-hidden="true" />
              </span>
            </label>
            <p className="item-path" id="plugin-project-path" title={project.directory}>{project.directory}</p>
          </div>
          {project.missing ? (
            <StatePanel icon={FolderKanban} title="Project file is missing" description="Select another project or return to Projects.">
              <button type="button" className="button secondary" onClick={onProjects}>Go to Projects</button>
            </StatePanel>
          ) : (
            <ProjectPlugins
              key={project.id}
              project={project}
              busy={busy}
              setBusy={setBusy}
              onNotice={(message) => hub.setNotice({ type: "success", message })}
              canBrowseLibrary={project.godotMajor !== null || hub.installed.data.some((engine) => engine.id === project.engineId)}
              onProjects={onProjects}
            />
          )}
        </>
      )}
    </div>
  );
}

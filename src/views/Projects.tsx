import { useCallback, useEffect, useState } from "react";
import {
  Box, ChevronDown, FolderKanban, FolderOpen, Import, LoaderCircle, Play, Plus, ScanSearch, Search,
} from "lucide-react";
import { api, architectureLabels, editionLabels, errorMessage, formatDate } from "../api";
import type { InstalledEngine, Project } from "../api";
import { ErrorNotice, LoadingState, Modal, StatePanel } from "../components";
import type { Hub } from "../useHub";

function engineLabel(engine: InstalledEngine) {
  const arch = architectureLabels[engine.arch] ?? engine.arch;
  return `Godot ${engine.tag} · ${editionLabels[engine.edition]} · ${arch}`;
}

function isCompatible(engine: InstalledEngine, project: Project) {
  const major = Number.parseInt(engine.tag, 10);
  return (project.godotMajor === null || project.godotMajor === major)
    && (!project.requiresDotnet || engine.edition === "dotnet");
}

function EnginePicker({
  value,
  onChange,
  engines,
  disabled,
  allowUnassigned = false,
  unavailableLabel,
}: {
  value: string;
  onChange: (id: string) => void;
  engines: InstalledEngine[];
  disabled: boolean;
  allowUnassigned?: boolean;
  unavailableLabel?: string;
}) {
  return (
    <label className="project-field">
      <span>Godot editor</span>
      <span className="select-field">
        <select value={value} onChange={(event) => onChange(event.target.value)} disabled={disabled} required={!allowUnassigned}>
          <option value="" disabled={!allowUnassigned}>{allowUnassigned ? "Unassigned" : "Choose an installed editor"}</option>
          {unavailableLabel && <option value={value} disabled>{unavailableLabel}</option>}
          {engines.map((engine) => {
            const label = engineLabel(engine);
            const duplicate = engines.some((other) => other.id !== engine.id && engineLabel(other) === label);
            return <option key={engine.id} value={engine.id} title={engine.executable}>
              {label}{engine.isDefault ? " (default)" : ""}{duplicate ? ` · ${engine.executable}` : ""}
            </option>;
          })}
        </select>
        <ChevronDown size={14} aria-hidden="true" />
      </span>
    </label>
  );
}

function CreateProjectDialog({
  engines,
  defaultDirectory,
  onClose,
  onBrowse,
  onCreate,
}: {
  engines: InstalledEngine[];
  defaultDirectory: string;
  onClose: () => void;
  onBrowse: () => void;
  onCreate: (project: Project) => void;
}) {
  const [name, setName] = useState("");
  const [directory, setDirectory] = useState(defaultDirectory);
  const [engineId, setEngineId] = useState(engines.find((engine) => engine.isDefault)?.id ?? "");
  const [busy, setBusy] = useState<"browse" | "create" | null>(null);
  const [error, setError] = useState("");
  const canCreate = name.trim() && directory.trim() && engines.some((engine) => engine.id === engineId);

  async function chooseFolder() {
    setBusy("browse");
    setError("");
    try {
      const path = await api.pickDirectory("Select the parent folder for the project", directory);
      if (path) setDirectory(path);
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(null);
    }
  }

  async function create() {
    if (!canCreate || busy) return;
    setBusy("create");
    setError("");
    try {
      onCreate(await api.createProject(name.trim(), directory.trim(), engineId));
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(null);
    }
  }

  return (
    <Modal title="New project" onClose={onClose} busy={Boolean(busy)}>
      {!engines.length ? (
        <>
          <p className="modal-copy">Install a Godot editor to create a project.</p>
          <div className="modal-actions">
            <button type="button" className="button primary" onClick={onBrowse}><Box size={16} /> Browse releases</button>
          </div>
        </>
      ) : (
        <form className="project-form" onSubmit={(event) => { event.preventDefault(); void create(); }}>
          {error && <ErrorNotice message={error} />}
          <label className="project-field">
            <span>Project name</span>
            <input autoFocus required value={name} onChange={(event) => setName(event.target.value)} disabled={Boolean(busy)} />
          </label>
          <div className="project-field">
            <label htmlFor="new-project-directory">Parent folder</label>
            <div className="directory-field">
              <input id="new-project-directory" required value={directory} onChange={(event) => setDirectory(event.target.value)} disabled={Boolean(busy)} aria-describedby="project-folder-hint" />
              <button type="button" className="button secondary" onClick={() => void chooseFolder()} disabled={Boolean(busy)}>
                {busy === "browse" ? <LoaderCircle className="spin" size={16} /> : <FolderOpen size={16} />} Browse
              </button>
            </div>
            <p id="project-folder-hint" className="modal-copy">A folder for the new project will be created here.</p>
          </div>
          <EnginePicker value={engineId} onChange={setEngineId} engines={engines} disabled={Boolean(busy)} />
          <div className="modal-actions">
            <button type="button" className="button secondary" onClick={onClose} disabled={Boolean(busy)}>Cancel</button>
            <button type="submit" className="button primary" disabled={!canCreate || Boolean(busy)}>
              {busy === "create" ? <LoaderCircle size={16} className="spin" /> : <Plus size={16} />} Create project
            </button>
          </div>
        </form>
      )}
    </Modal>
  );
}

function AssignEngineDialog({
  project,
  engines,
  onClose,
  onBrowse,
  onAssign,
}: {
  project: Project;
  engines: InstalledEngine[];
  onClose: () => void;
  onBrowse: () => void;
  onAssign: (project: Project) => void;
}) {
  const [engineId, setEngineId] = useState(project.engineId ?? "");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  const compatible = engines.filter((engine) => engine.available !== false && isCompatible(engine, project));
  const engine = engines.find((item) => item.id === engineId);
  const unavailable = engineId && !compatible.some((item) => item.id === engineId);
  const version = project.godotMajor === null ? "Godot" : `Godot ${project.godotMajor}.x`;
  const requirement = `${version}${project.requiresDotnet ? " .NET" : ""}`;

  async function save() {
    if (saving || unavailable || engineId === (project.engineId ?? "")) return;
    setSaving(true);
    setError("");
    try {
      onAssign(await api.setProjectEngine(project.id, engineId || null));
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setSaving(false);
    }
  }

  return (
    <Modal title={`Editor for ${project.name}`} onClose={onClose} busy={saving}>
      <form className="project-form" onSubmit={(event) => { event.preventDefault(); void save(); }}>
        {error && <ErrorNotice message={error} />}
        <p className="modal-copy">Choose an installed {requirement} editor for this project.</p>
        <EnginePicker
          value={engineId}
          onChange={setEngineId}
          engines={compatible}
          disabled={saving}
          allowUnassigned
          unavailableLabel={unavailable ? engine ? `${engineLabel(engine)} (unavailable)` : "Previously assigned editor (unavailable)" : undefined}
        />
        {!compatible.length && (
          <div className="project-install-hint">
            <p className="modal-copy">No compatible editors installed.</p>
            <button type="button" className="button secondary small" onClick={onBrowse} disabled={saving}>Browse releases</button>
          </div>
        )}
        <div className="modal-actions">
          <button type="button" className="button secondary" onClick={onClose} disabled={saving}>Cancel</button>
          <button type="submit" className="button primary" disabled={saving || Boolean(unavailable) || engineId === (project.engineId ?? "")}>
            {saving && <LoaderCircle size={16} className="spin" />} Save editor
          </button>
        </div>
      </form>
    </Modal>
  );
}

export function Projects({ hub, onBrowse }: { hub: Hub; onBrowse: () => void }) {
  const [projects, setProjects] = useState<Project[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState("");
  const [creating, setCreating] = useState(false);
  const [assigning, setAssigning] = useState<Project | null>(null);
  const [query, setQuery] = useState("");
  const engines = hub.installed.data;
  const readyEngines = engines.filter((engine) => engine.available && /^[234]\./.test(engine.tag));
  const engineListReady = !hub.installed.loading && !hub.installed.error;
  const defaultDirectory = hub.settings.data?.projectDirectory ?? "";
  const canStartCreate = !busy && !loading && !error && engineListReady && !hub.settings.loading;

  const loadProjects = useCallback(async (showLoading = true, discover = false) => {
    if (showLoading) setLoading(true);
    setError("");
    try {
      const result = discover
        ? await api.discoverProjects().then(({ projects }) => projects).catch(() => api.projects())
        : await api.projects();
      setProjects(result);
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { void loadProjects(true, true); }, [loadProjects]);

  async function perform(key: string, action: () => Promise<void>) {
    setBusy(key);
    try {
      await action();
    } catch (cause) {
      hub.setNotice({ type: "error", message: errorMessage(cause) });
      if (key !== "import") await Promise.all([loadProjects(false), hub.loadInstalled(false)]);
    } finally {
      setBusy("");
    }
  }

  async function importProject() {
    await perform("import", async () => {
      const file = await api.pickProject(defaultDirectory || undefined);
      if (!file) return;
      const project = await api.importProject(file);
      hub.setNotice({
        type: "success",
        message: projects.some((item) => item.id === project.id) ? `${project.name} is already in Projects.` : `${project.name} imported.`,
      });
      await loadProjects(false);
    });
  }

  async function scanFolder() {
    await perform("scan", async () => {
      const directory = await api.pickDirectory("Select a folder containing Godot projects", defaultDirectory);
      if (!directory) return;
      const result = await api.discoverProjects(directory);
      setProjects(result.projects);
      setQuery("");
      const found = result.added ? `Added ${result.added} project${result.added === 1 ? "" : "s"}.` : "No new Godot projects found.";
      hub.setNotice({ type: "success", message: result.limited ? `${found} Select a smaller folder to scan deeper.` : found });
    });
  }

  function createCompleted(project: Project) {
    setCreating(false);
    hub.setNotice({ type: "success", message: `${project.name} created.` });
    void loadProjects(false);
  }

  function assignmentCompleted(project: Project) {
    setAssigning(null);
    hub.setNotice({ type: "success", message: project.engineId ? `Editor saved for ${project.name}.` : `${project.name} is unassigned.` });
    void loadProjects(false);
  }

  function browseFromDialog() {
    setCreating(false);
    setAssigning(null);
    onBrowse();
  }

  const search = query.trim().toLowerCase();
  const matchingProjects = projects.filter((project) =>
    project.name.toLowerCase().includes(search) || project.directory.toLowerCase().includes(search));

  return (
    <>
      <header className="page-heading">
        <div>
          <h1>Projects</h1>
          <p>Create projects and open them with the Godot editor you choose.</p>
        </div>
        <div className="heading-actions">
          <button type="button" className="button secondary" onClick={() => void scanFolder()} disabled={Boolean(busy) || loading || hub.settings.loading}>
            {busy === "scan" ? <LoaderCircle size={17} className="spin" /> : <ScanSearch size={17} />} Scan folder
          </button>
          <button type="button" className="button secondary" onClick={() => void importProject()} disabled={Boolean(busy) || loading || Boolean(error)}>
            {busy === "import" ? <LoaderCircle size={17} className="spin" /> : <Import size={17} />} Import project
          </button>
          <button type="button" className="button primary" onClick={() => setCreating(true)} disabled={!canStartCreate}>
            <Plus size={17} /> New project
          </button>
        </div>
      </header>
      <section className="view-section" aria-label="Recent projects">
        {error && <ErrorNotice message={error} retry={() => void loadProjects()} />}
        {hub.installed.error && hub.installed.error !== error && <ErrorNotice message={hub.installed.error} retry={() => void hub.loadInstalled()} />}
        {loading && !projects.length && <LoadingState title="Loading projects" />}
        {!loading && !error && !projects.length && (
          <StatePanel icon={FolderKanban} title="No projects yet" description="Use Scan folder to find your existing Godot projects, or create a new project." />
        )}
        {projects.length > 0 && (
          <>
            <h2 className="section-title">Recent projects</h2>
            <label className="search-field project-search">
              <Search size={17} aria-hidden="true" />
              <input type="search" aria-label="Search projects" placeholder="Search by name or path" value={query} onChange={(event) => setQuery(event.target.value)} />
            </label>
            {!matchingProjects.length && <p className="modal-copy">No projects match your search.</p>}
            <ul className="item-list">
              {matchingProjects.map((project) => {
                const engine = engines.find((item) => item.id === project.engineId);
                const validEngine = engine && engine.available !== false && isCompatible(engine, project);
                const canOpen = engineListReady && validEngine && !project.missing;
                const unavailableEditor = engine && engine.available !== false
                  ? "Assigned editor is incompatible"
                  : "Assigned editor unavailable";
                const editorStatus = validEngine
                  ? engineLabel(engine)
                  : project.engineId ? (hub.installed.loading ? "Checking assigned editor…" : unavailableEditor) : "No editor assigned";
                const openHint = project.missing ? "Project file is missing" : !engineListReady ? "Editor list unavailable" : !validEngine ? editorStatus : `Open ${project.name}`;
                const opening = busy === `open-${project.id}`;
                const editorLabel = project.engineId ? "Change editor" : "Choose editor";
                return (
                  <li className="item-row project-row" key={project.id}>
                    <span className="version-icon"><FolderKanban size={21} aria-hidden="true" /></span>
                    <div className="item-content">
                      <div className="item-title">
                        <h3>{project.name}</h3>
                        {project.missing && <span className="pill project-issue">Missing project file</span>}
                      </div>
                      <p className="item-path" title={project.directory}>{project.directory}</p>
                      <p className="item-meta">
                        <span className={!validEngine ? "project-editor-issue" : undefined}>{editorStatus}</span>
                        {project.lastOpenedAt !== null
                          ? <span>Opened {formatDate(project.lastOpenedAt)}</span>
                          : <span>Added {formatDate(project.addedAt)}</span>}
                      </p>
                    </div>
                    <div className="row-actions">
                      <button type="button" className="button secondary" aria-label={`${editorLabel} for ${project.name}`} onClick={() => setAssigning(project)} disabled={Boolean(busy) || !engineListReady}>
                        {editorLabel}
                      </button>
                      <button type="button" className="button secondary icon-button" title="Show in File Explorer" aria-label={`Show ${project.name} in File Explorer`}
                        onClick={() => void perform(`reveal-${project.id}`, () => api.revealProject(project.id))} disabled={Boolean(busy)}>
                        <FolderOpen size={17} aria-hidden="true" />
                      </button>
                      <button type="button" className="button primary" title={openHint} aria-label={`Open ${project.name}`} disabled={!canOpen || Boolean(busy)}
                        onClick={() => void perform(`open-${project.id}`, async () => {
                          const result = await api.launchProject(project.id);
                          hub.setNotice({ type: "success", message: `${result.name} opened.` });
                          await loadProjects(false);
                        })}>
                        {opening ? <LoaderCircle size={16} className="spin" /> : <Play size={16} />} Open
                      </button>
                    </div>
                  </li>
                );
              })}
            </ul>
          </>
        )}
      </section>
      {creating && <CreateProjectDialog engines={readyEngines} defaultDirectory={defaultDirectory} onClose={() => setCreating(false)} onBrowse={browseFromDialog} onCreate={createCompleted} />}
      {assigning && <AssignEngineDialog project={assigning} engines={engines} onClose={() => setAssigning(null)} onBrowse={browseFromDialog} onAssign={assignmentCompleted} />}
    </>
  );
}

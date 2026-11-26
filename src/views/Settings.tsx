import { useState } from "react";
import { Bell, Download, Folder, FolderOpen, HardDrive, LoaderCircle, Moon, RefreshCw, Sun } from "lucide-react";
import { api, errorMessage, formatSize } from "../api";
import type { Settings as HubSettings, Theme } from "../api";
import { ErrorNotice, LoadingState } from "../components";
import type { Hub } from "../useHub";
import type { AppUpdater } from "../useAppUpdater";

function DirectoryField({
  name,
  label,
  description,
  value,
  onChange,
  icon: Icon,
  busy,
}: {
  name: string;
  label: string;
  description: string;
  value: string;
  onChange: (value: string) => void;
  icon: typeof Folder;
  busy: boolean;
}) {
  const [picking, setPicking] = useState(false);
  const [error, setError] = useState("");

  async function choose() {
    setError("");
    setPicking(true);
    try {
      const path = await api.pickDirectory(`Select ${label.toLowerCase()}`, value);
      if (path) onChange(path);
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setPicking(false);
    }
  }

  return (
    <div className="setting-row directory-row">
      <span className="settings-icon"><Icon size={18} aria-hidden="true" /></span>
      <div className="setting-content">
        <label htmlFor={name}>{label}</label>
        <p id={`${name}-hint`}>{description}</p>
        <div className="directory-field">
          <input id={name} value={value} required disabled={busy} aria-describedby={`${name}-hint`} onChange={(event) => onChange(event.target.value)} />
          <button type="button" className="button secondary" disabled={busy || picking} onClick={() => void choose()} aria-label={`Browse for ${label.toLowerCase()}`}>
            {picking ? <LoaderCircle size={16} className="spin" /> : <FolderOpen size={16} />}
            Browse
          </button>
        </div>
        {error && <ErrorNotice message={error} />}
      </div>
    </div>
  );
}

function SettingsForm({ initial, hub }: { initial: HubSettings; hub: Hub }) {
  const [draft, setDraft] = useState(initial);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  const dirty = draft.engineDirectory !== initial.engineDirectory || draft.projectDirectory !== initial.projectDirectory ||
    draft.autoCheckUpdates !== initial.autoCheckUpdates || draft.autoUpdateHub !== initial.autoUpdateHub || draft.theme !== initial.theme;

  function update<K extends keyof HubSettings>(key: K, value: HubSettings[K]) {
    setDraft((current) => ({ ...current, [key]: value }));
  }

  async function save() {
    if (!dirty || saving) return;
    setSaving(true);
    setError("");
    try {
      const result = await api.saveSettings(draft);
      setDraft(result);
      hub.applySettings(result);
      hub.setNotice({ type: "success", message: "Settings saved." });
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setSaving(false);
    }
  }

  const themes: { value: Theme; icon: typeof Moon; label: string }[] = [
    { value: "dark", icon: Moon, label: "Dark" },
    { value: "light", icon: Sun, label: "Light" },
  ];

  return (
    <form className="settings-form" onSubmit={(event) => { event.preventDefault(); void save(); }}>
      <section className="settings-group" aria-label="Storage settings">
        <h2 className="section-title">Storage</h2>
        <div className="settings-card">
          <DirectoryField name="engine-dir" label="Engine installation folder" description="New engine versions are installed here." value={draft.engineDirectory} onChange={(path) => update("engineDirectory", path)} icon={HardDrive} busy={saving} />
          <DirectoryField name="project-dir" label="Default project folder" description="Preferred location for new projects." value={draft.projectDirectory} onChange={(path) => update("projectDirectory", path)} icon={Folder} busy={saving} />
        </div>
      </section>
      <section className="settings-group" aria-label="General settings">
        <h2 className="section-title">General</h2>
        <div className="settings-card">
          <div className="setting-row">
            <span className="settings-icon"><Bell size={18} aria-hidden="true" /></span>
            <div className="setting-content">
              <p className="setting-label" id="update-setting">Check for engine updates at startup</p>
              <p id="update-setting-description">Check GitHub for updates to your installed engines when Godot Hub opens.</p>
            </div>
            <button
              type="button"
              className="toggle"
              role="switch"
              aria-checked={draft.autoCheckUpdates}
              aria-labelledby="update-setting"
              aria-describedby="update-setting-description"
              disabled={saving}
              onClick={() => update("autoCheckUpdates", !draft.autoCheckUpdates)}
            >
              <span />
            </button>
          </div>
          <div className="setting-row">
            <span className="settings-icon"><Download size={18} aria-hidden="true" /></span>
            <div className="setting-content">
              <p className="setting-label" id="hub-update-setting">Automatically update Godot Hub</p>
              <p id="hub-update-setting-description">Download verified app updates at startup and install them when downloads and project operations finish.</p>
            </div>
            <button type="button" className="toggle" role="switch" aria-checked={draft.autoUpdateHub}
              aria-labelledby="hub-update-setting" aria-describedby="hub-update-setting-description" disabled={saving}
              onClick={() => update("autoUpdateHub", !draft.autoUpdateHub)}>
              <span />
            </button>
          </div>
          <div className="setting-row">
            <span className="settings-icon"><Sun size={18} aria-hidden="true" /></span>
            <div className="setting-content">
              <p className="setting-label" id="appearance-setting">Appearance</p>
              <p>Choose how Godot Hub looks.</p>
            </div>
            <div className="segmented" role="group" aria-labelledby="appearance-setting">
              {themes.map(({ value, label, icon: Icon }) => (
                <button key={value} type="button" aria-pressed={draft.theme === value} disabled={saving} onClick={() => update("theme", value)}>
                  <Icon size={15} aria-hidden="true" /> {label}
                </button>
              ))}
            </div>
          </div>
        </div>
      </section>
      {error && <ErrorNotice message={error} />}
      <div className="settings-footer">
        <button type="button" className="button secondary" disabled={!dirty || saving} onClick={() => { setDraft(initial); setError(""); }}>Reset changes</button>
        <button type="submit" className="button primary" disabled={!dirty || saving}>
          {saving && <LoaderCircle size={16} className="spin" />}
          Save settings
        </button>
      </div>
    </form>
  );
}

function HubUpdates({ updater }: { updater: AppUpdater }) {
  const { stage, version, candidate, progress, error } = updater;
  const running = stage === "checking" || stage === "downloading" || stage === "installing";
  const descriptions = {
    idle: "Check GitHub for a Godot Hub update.",
    checking: "Checking for updates…",
    current: "Godot Hub is up to date.",
    ready: candidate ? `Godot Hub ${candidate.version} is available.` : "An update is available.",
    waiting: "The update is waiting for active work to finish.",
    downloading: progress ? `Downloaded ${formatSize(progress.downloaded)}${progress.total ? ` of ${formatSize(progress.total)}` : ""}.` : "Starting the update download…",
    installing: "Installing the update. Godot Hub will restart.",
    error: "The last update check or installation could not finish.",
  };
  return (
    <section className="settings-group hub-updates" aria-label="Godot Hub updates">
      <h2 className="section-title">Godot Hub updates</h2>
      <div className="settings-card">
        <div className="setting-row">
          <span className="settings-icon"><RefreshCw size={18} className={running ? "spin" : undefined} aria-hidden="true" /></span>
          <div className="setting-content">
            <p className="setting-label">{version ? `Installed version ${version}` : "Installed version"}</p>
            <p role="status">{descriptions[stage]}</p>
            {stage === "downloading" && <progress aria-label="Godot Hub update download" max={progress?.total ?? 1} value={progress?.total ? progress.downloaded : undefined} />}
          </div>
          <div className="row-actions">
            {candidate && !running && <button type="button" className="button primary" disabled={updater.activeDownloads} onClick={() => void updater.install()}>Install update</button>}
            <button type="button" className="button secondary" disabled={running} onClick={() => void updater.check()}>Check now</button>
          </div>
        </div>
        {error && <ErrorNotice message={error} />}
      </div>
    </section>
  );
}

export function Settings({ hub, updater }: { hub: Hub; updater: AppUpdater }) {
  return (
    <>
      <header className="page-heading">
        <div>
          <h1>Settings</h1>
          <p>Set storage locations and choose your preferences.</p>
        </div>
      </header>
      <div className="view-section">
        {hub.settings.error && <ErrorNotice message={hub.settings.error} retry={() => void hub.loadSettings()} />}
        {hub.settings.loading && !hub.settings.data && <LoadingState title="Loading settings" />}
        {hub.settings.data && <SettingsForm initial={hub.settings.data} hub={hub} />}
        <HubUpdates updater={updater} />
      </div>
    </>
  );
}

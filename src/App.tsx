import { useEffect, useRef, useState } from "react";
import { ArrowUpRight, Box, CheckCircle, CircleAlert, Download, FolderKanban, Monitor, Settings2, X } from "lucide-react";
import { architectureLabels, isActive } from "./api";
import { StatePanel } from "./components";
import { useHub } from "./useHub";
import { Downloads } from "./views/Downloads";
import { Installs } from "./views/Installs";
import type { InstallsTab } from "./views/Installs";
import { Settings } from "./views/Settings";

const navigation = [
  { name: "Projects", id: "projects", icon: FolderKanban },
  { name: "Installs", id: "installs", icon: Box },
  { name: "Downloads", id: "downloads", icon: Download },
  { name: "Settings", id: "settings", icon: Settings2 },
] as const;

type View = typeof navigation[number]["id"];

export function App() {
  const [view, setView] = useState<View>("installs");
  const [installsTab, setInstallsTab] = useState<InstallsTab>("installed");
  const main = useRef<HTMLElement>(null);
  const hub = useHub();
  const activeCount = hub.jobs.filter(isActive).length;

  useEffect(() => {
    main.current?.scrollTo(0, 0);
  }, [view, installsTab]);

  function browseReleases() {
    setInstallsTab("browse");
    setView("installs");
  }

  return (
    <div className="app-shell">
      <aside className="sidebar">
        <div className="brand">
          <img src="/hub-icon.png" alt="" width={40} height={40} />
          <div><strong>Godot Hub</strong><span>Engine manager</span></div>
        </div>
        <nav aria-label="Main navigation" className="navigation">
          {navigation.map(({ name, id, icon: Icon }) => (
            <button key={id} type="button" aria-label={id === "downloads" && activeCount ? `Downloads, ${activeCount} active` : name} title={name} aria-current={view === id ? "page" : undefined} onClick={() => setView(id)}>
              <Icon size={19} strokeWidth={1.8} aria-hidden="true" />
              <span>{name}</span>
              {id === "downloads" && activeCount > 0 && <span className="nav-count" aria-label={`${activeCount} active downloads`}>{activeCount}</span>}
            </button>
          ))}
        </nav>
        <div className="sidebar-footer">
          <Monitor size={16} aria-hidden="true" />
          <span>Windows{hub.architecture && ` · ${architectureLabels[hub.architecture]}`}</span>
        </div>
      </aside>
      <main className="main-surface" ref={main}>
        <div hidden={view !== "installs"}>
          <Installs hub={hub} tab={installsTab} setTab={setInstallsTab} />
        </div>
        {view === "downloads" && <Downloads hub={hub} onBrowse={browseReleases} />}
        {view === "settings" && <Settings hub={hub} />}
        {view === "projects" && (
          <>
            <header className="page-heading"><div><h1>Projects</h1></div></header>
            <div className="view-section">
              <StatePanel icon={FolderKanban} title="Project management is planned" description="Use Installs to set up a Godot editor and open projects from the editor.">
                <button type="button" className="button secondary" onClick={() => { setInstallsTab("installed"); setView("installs"); }}>
                  <Box size={17} /> Go to Installs
                </button>
              </StatePanel>
            </div>
          </>
        )}
      </main>
      {hub.notice && (
        <div className={`toast toast-${hub.notice.type}`} role={hub.notice.type === "error" ? "alert" : "status"}>
          {hub.notice.type === "error" ? <CircleAlert size={20} /> : <CheckCircle size={20} />}
          <div>
            <p>{hub.notice.message}</p>
            {hub.notice.action && (
              <button type="button" className="text-button" onClick={() => {
                if (hub.notice?.action === "updates") {
                  setInstallsTab("updates");
                  setView("installs");
                } else {
                  setView("downloads");
                }
                hub.setNotice(null);
              }}>
                {hub.notice.action === "updates" ? "View updates" : "View downloads"} <ArrowUpRight size={15} />
              </button>
            )}
          </div>
          <button type="button" className="button icon-button ghost small" aria-label="Dismiss notification" onClick={() => hub.setNotice(null)}><X size={16} /></button>
        </div>
      )}
    </div>
  );
}

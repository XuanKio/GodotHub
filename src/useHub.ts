import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, errorMessage, isActive } from "./api";
import type { Architecture, Catalogue, Edition, InstalledEngine, Job, Settings, Stage } from "./api";
import { findEngineUpdates, updateKey } from "./updates";

type Resource<T> = { data: T; loading: boolean; error: string };
type Notice = { message: string; type: "success" | "error"; action?: "downloads" | "updates" };
/** An editor being installed for a project, assigned to it once the install completes. */
export type EditorRequest = { tag: string; edition: Edition; arch: string; jobId: string };
type CatalogueResult = Omit<Resource<Catalogue | null>, "loading">;

const stageOrder: Record<Stage, number> = {
  queued: 0,
  downloading: 1,
  verifying: 2,
  extracting: 3,
  installing: 4,
  completed: 5,
  failed: 5,
};

function mergeJobs(current: Job[], incoming: Job[]) {
  const jobsById = new Map(current.map((job) => [job.id, job]));
  incoming.forEach((job) => {
    const previous = jobsById.get(job.id);
    if (!previous || stageOrder[job.stage] > stageOrder[previous.stage] ||
      (job.stage === previous.stage && job.downloaded >= previous.downloaded)) {
      jobsById.set(job.id, job);
    }
  });
  return Array.from(jobsById.values()).sort((first, second) => second.startedAt - first.startedAt);
}

export function useHub() {
  const [settings, setSettings] = useState<Resource<Settings | null>>({ data: null, loading: true, error: "" });
  const [installed, setInstalled] = useState<Resource<InstalledEngine[]>>({ data: [], loading: true, error: "" });
  const [catalogue, setCatalogue] = useState<Resource<Catalogue | null>>({ data: null, loading: false, error: "" });
  const [updateCheck, setUpdateCheck] = useState<Resource<Catalogue | null>>({ data: null, loading: false, error: "" });
  const [architecture, setArchitecture] = useState<Architecture | null>(null);
  const [jobs, setJobs] = useState<Job[]>([]);
  const [jobsLoading, setJobsLoading] = useState(true);
  const [jobsError, setJobsError] = useState("");
  const [notice, setNotice] = useState<Notice | null>(null);
  const [editorRequests, setEditorRequests] = useState<Record<string, EditorRequest>>({});
  const catalogueRequest = useRef<{ force: boolean; promise: Promise<CatalogueResult> } | null>(null);
  const checkingUpdates = useRef(false);
  const announcedUpdates = useRef(new Set<string>());
  const finishedJobs = useRef(new Set<string>());
  const checkedAtStartup = useRef(false);

  const loadSettings = useCallback(async () => {
    setSettings((current) => ({ ...current, loading: true, error: "" }));
    try {
      setSettings({ data: await api.settings(), loading: false, error: "" });
    } catch (error) {
      setSettings((current) => ({ ...current, loading: false, error: errorMessage(error) }));
    }
  }, []);

  const loadInstalled = useCallback(async (showLoading = true) => {
    if (showLoading) setInstalled((current) => ({ ...current, loading: true, error: "" }));
    try {
      setInstalled({ data: await api.installed(), loading: false, error: "" });
    } catch (error) {
      setInstalled((current) => ({ ...current, loading: false, error: errorMessage(error) }));
    }
  }, []);

  const loadCatalogue = useCallback(function load(force = false): Promise<CatalogueResult> {
    const pending = catalogueRequest.current;
    if (pending) {
      return force && !pending.force ? pending.promise.then(() => load(true)) : pending.promise;
    }
    setCatalogue((current) => ({ ...current, loading: true, error: "" }));
    const promise = api.releases(force)
      .then<CatalogueResult, CatalogueResult>(
        (data) => ({ data, error: "" }),
        (error: unknown) => ({ data: null, error: errorMessage(error) }),
      )
      .then((result) => {
        setCatalogue((current) => ({ ...result, data: result.data ?? current.data, loading: false }));
        return result;
      })
      .finally(() => { catalogueRequest.current = null; });
    catalogueRequest.current = { force, promise };
    return promise;
  }, []);

  const checkUpdates = useCallback(async () => {
    if (checkingUpdates.current) return;
    checkingUpdates.current = true;
    setUpdateCheck((current) => ({ ...current, loading: true, error: "" }));
    const result = await loadCatalogue(true);
    setUpdateCheck((current) => ({ ...result, data: result.data ?? current.data, loading: false }));
    checkingUpdates.current = false;
  }, [loadCatalogue]);

  const checkedReleases = updateCheck.data?.releases;
  const updates = useMemo(() => checkedReleases ? findEngineUpdates(installed.data, checkedReleases) : [], [installed.data, checkedReleases]);

  const recordJob = useCallback((job: Job) => {
    setJobs((current) => mergeJobs(current, [job]));
  }, []);

  const loadJobs = useCallback(async () => {
    try {
      const current = await api.jobs();
      setJobs((previous) => mergeJobs(previous, current));
      setJobsError("");
    } catch (error) {
      setJobsError(errorMessage(error));
    } finally {
      setJobsLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadSettings();
    void loadInstalled();
    void api.platform().then(setArchitecture).catch(() => setArchitecture(null));
    let disposed = false;
    const listener = listen<Job>("hub://job", ({ payload }) => recordJob(payload));
    void listener.catch(() => undefined).then(() => {
      if (!disposed) void loadJobs();
    });
    return () => {
      disposed = true;
      void listener.then((unlisten) => unlisten()).catch(() => undefined);
    };
  }, [loadInstalled, loadJobs, loadSettings, recordJob]);

  useEffect(() => {
    if (!settings.data || checkedAtStartup.current) return;
    checkedAtStartup.current = true;
    if (settings.data.autoCheckUpdates) void checkUpdates();
  }, [settings.data, checkUpdates]);

  useEffect(() => {
    if (installed.loading || installed.error || updateCheck.loading || updateCheck.error || updateCheck.data?.stale) return;
    const fresh = updates.filter((update) => !announcedUpdates.current.has(updateKey(update)));
    if (!fresh.length) return;
    fresh.forEach((update) => announcedUpdates.current.add(updateKey(update)));
    setNotice({
      type: "success",
      message: fresh.length === 1 ? `Godot ${fresh[0].release.tag} is available.` : `${fresh.length} Godot updates are available.`,
      action: "updates",
    });
  }, [installed.loading, installed.error, updateCheck.loading, updateCheck.error, updateCheck.data?.stale, updates]);

  const hasActiveJobs = jobs.some(isActive);

  useEffect(() => {
    if (!hasActiveJobs) return;
    const timer = window.setInterval(() => void loadJobs(), 1500);
    return () => window.clearInterval(timer);
  }, [hasActiveJobs, loadJobs]);

  useEffect(() => {
    const finished = jobs.filter((job) => !isActive(job) && !finishedJobs.current.has(job.id));
    if (!finished.length) return;
    finished.forEach((job) => finishedJobs.current.add(job.id));
    if (finished.some((job) => job.stage === "completed")) void loadInstalled(false);
    const failed = finished.find((job) => job.stage === "failed");
    setNotice(failed
      ? { type: "error", message: `${failed.label} could not be installed.`, action: "downloads" }
      : { type: "success", message: `${finished[0].label} installed.`, action: "downloads" });
  }, [jobs, loadInstalled]);

  useEffect(() => {
    document.documentElement.dataset.theme = settings.data?.theme ?? "dark";
  }, [settings.data?.theme]);

  useEffect(() => {
    if (notice?.type !== "success") return;
    const timer = window.setTimeout(() => setNotice(null), 6500);
    return () => window.clearTimeout(timer);
  }, [notice]);

  const requestEditor = useCallback((projectId: string, request: EditorRequest | null) => {
    setEditorRequests((current) => {
      const next = { ...current };
      if (request) next[projectId] = request;
      else delete next[projectId];
      return next;
    });
  }, []);

  function applySettings(data: Settings) {
    setSettings({ data, loading: false, error: "" });
  }

  return {
    settings,
    applySettings,
    loadSettings,
    installed,
    loadInstalled,
    catalogue,
    loadCatalogue,
    updates,
    updateCheck,
    checkUpdates,
    architecture,
    jobs,
    jobsLoading,
    jobsError,
    loadJobs,
    recordJob,
    notice,
    setNotice,
    editorRequests,
    requestEditor,
  };
}

export type Hub = ReturnType<typeof useHub>;

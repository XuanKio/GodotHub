import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";
import { api, errorMessage } from "./api";
import type { AppUpdate, AppUpdateProgress } from "./api";

type Stage = "idle" | "checking" | "current" | "ready" | "waiting" | "downloading" | "installing" | "error";

export function useAppUpdater(automatic: boolean | undefined, activeDownloads: boolean) {
  const [version, setVersion] = useState("");
  const [candidate, setCandidate] = useState<AppUpdate | null>(null);
  const [stage, setStage] = useState<Stage>("idle");
  const [progress, setProgress] = useState<AppUpdateProgress | null>(null);
  const [error, setError] = useState("");
  const inFlight = useRef(false);
  const started = useRef(false);

  useEffect(() => {
    void api.hubVersion().then(setVersion).catch(() => undefined);
    const listener = listen<AppUpdateProgress>("hub://app-update", ({ payload }) => {
      setProgress(payload);
      setStage(payload.stage);
    });
    void listener.catch(() => undefined);
    return () => { void listener.then((unlisten) => unlisten()).catch(() => undefined); };
  }, []);

  const check = useCallback(async () => {
    if (inFlight.current) return;
    inFlight.current = true;
    setStage("checking");
    setError("");
    setProgress(null);
    try {
      const available = await api.checkAppUpdate();
      setCandidate(available);
      setStage(available ? "ready" : "current");
    } catch (cause) {
      setCandidate(null);
      setError(errorMessage(cause));
      setStage("error");
    } finally {
      inFlight.current = false;
    }
  }, []);

  const install = useCallback(async () => {
    if (!candidate || inFlight.current) return;
    inFlight.current = true;
    setStage("downloading");
    setProgress(null);
    setError("");
    try {
      const started = await api.installAppUpdate(candidate.version);
      setStage(started ? "installing" : "waiting");
    } catch (cause) {
      setError(errorMessage(cause));
      setStage("error");
    } finally {
      inFlight.current = false;
    }
  }, [candidate]);

  useEffect(() => {
    if (!automatic || started.current) return;
    started.current = true;
    void check();
  }, [automatic, check]);

  useEffect(() => {
    if (!automatic || !candidate || activeDownloads || (stage !== "ready" && stage !== "waiting")) return;
    const timer = window.setInterval(() => {
      if (!document.querySelector("dialog[open]")) void install();
    }, stage === "ready" ? 1000 : 5000);
    return () => window.clearInterval(timer);
  }, [automatic, activeDownloads, candidate, stage, install]);

  return { version, candidate, stage, progress, error, check, install, activeDownloads };
}

export type AppUpdater = ReturnType<typeof useAppUpdater>;

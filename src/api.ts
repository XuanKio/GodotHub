import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";

export type Edition = "standard" | "dotnet";
export type Architecture = "x64" | "arm64" | "x86";
export type Channel = "stable" | "beta" | "rc" | "dev";
export type Theme = "dark" | "light";

export interface Settings {
  engineDirectory: string;
  projectDirectory: string;
  autoCheckUpdates: boolean;
  theme: Theme;
}

export interface InstalledEngine {
  id: string;
  tag: string;
  edition: Edition;
  arch: string;
  executable: string;
  managed: boolean;
  isDefault: boolean;
  templatesInstalled: boolean;
  installedAt: number;
  available: boolean;
}

export interface Project {
  id: string;
  name: string;
  directory: string;
  projectFile: string;
  engineId: string | null;
  addedAt: number;
  lastOpenedAt: number | null;
  godotMajor: number | null;
  requiresDotnet: boolean;
  missing: boolean;
}

export interface Asset {
  name: string;
  url: string;
  size: number;
  digest?: string | null;
  checksumUrl?: string | null;
}

export interface Build {
  edition: Edition;
  arch: Architecture;
  engine: Asset;
  templates?: Asset | null;
}

export interface Release {
  tag: string;
  channel: Channel;
  notes: string;
  publishedAt: string;
  url: string;
  builds: Build[];
}

export interface Catalogue {
  releases: Release[];
  stale: boolean;
  warning?: string | null;
}

export type Stage = "queued" | "downloading" | "verifying" | "extracting" |
  "installing" | "completed" | "failed";

export interface Job {
  id: string;
  label: string;
  kind: "engine" | "templates";
  stage: Stage;
  downloaded: number;
  total: number;
  error?: string | null;
  startedAt: number;
}

export const api = {
  settings: () => invoke<Settings>("get_settings"),
  saveSettings: (settings: Settings) => invoke<Settings>("save_settings", { settings }),
  platform: () => invoke<Architecture>("get_platform"),
  installed: () => invoke<InstalledEngine[]>("list_installed"),
  releases: (force: boolean) => invoke<Catalogue>("list_releases", { force }),
  jobs: () => invoke<Job[]>("list_jobs"),
  install: (tag: string, edition: Edition, arch: Architecture) =>
    invoke<Job>("install_engine", { tag, edition, arch }),
  templates: (id: string) => invoke<Job>("install_templates", { id }),
  importEngine: (executable: string) =>
    invoke<InstalledEngine>("import_engine", { executable }),
  launch: (id: string) => invoke<void>("launch_engine", { id }),
  remove: (id: string) => invoke<void>("remove_engine", { id }),
  setDefault: (id: string) => invoke<void>("set_default_engine", { id }),
  shortcut: (id: string) => invoke<string>("create_engine_shortcut", { id }),
  reveal: (id: string) => invoke<void>("reveal_engine", { id }),
  openRelease: (tag: string) => invoke<void>("open_release_page", { tag }),
  openChangelog: (tag: string, curated: boolean) => invoke<void>("open_changelog", { tag, curated }),
  projects: () => invoke<Project[]>("list_projects"),
  createProject: (name: string, parentDirectory: string, engineId: string) =>
    invoke<Project>("create_project", { name, parentDirectory, engineId }),
  importProject: (projectFile: string) => invoke<Project>("import_project", { projectFile }),
  setProjectEngine: (id: string, engineId: string | null) =>
    invoke<Project>("set_project_engine", { id, engineId }),
  launchProject: (id: string) => invoke<Project>("launch_project", { id }),
  revealProject: (id: string) => invoke<void>("reveal_project", { id }),
  pickEngine: () => open({
    title: "Select the Godot editor executable",
    multiple: false,
    filters: [{ name: "Windows executable", extensions: ["exe"] }],
  }),
  pickProject: (defaultPath?: string) => open({
    title: "Select project.godot or engine.cfg",
    defaultPath,
    multiple: false,
    filters: [{ name: "Godot project files", extensions: ["godot", "cfg"] }],
  }),
  pickDirectory: (title: string, defaultPath: string) => open({
    title,
    defaultPath,
    directory: true,
    multiple: false,
  }),
};

export const editionLabels: Record<Edition, string> = {
  standard: "Standard",
  dotnet: ".NET",
};

export const architectureLabels: Record<string, string> = {
  x64: "x64",
  arm64: "ARM64",
  x86: "x86",
};

export const channelLabels: Record<Channel, string> = {
  stable: "Stable",
  beta: "Beta",
  rc: "RC",
  dev: "Dev",
};

export function isActive(job: Job) {
  return job.stage !== "completed" && job.stage !== "failed";
}

export function formatSize(bytes: number) {
  if (bytes >= 1024 ** 3) return `${(bytes / 1024 ** 3).toFixed(1)} GB`;
  if (bytes >= 1024 ** 2) return `${(bytes / 1024 ** 2).toFixed(1)} MB`;
  if (bytes >= 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${bytes} B`;
}

export function formatDate(date: string | number) {
  const value = new Date(date);
  if (Number.isNaN(value.getTime())) return "";
  return new Intl.DateTimeFormat(undefined, { dateStyle: "medium" }).format(value);
}

export function errorMessage(error: unknown) {
  if (error instanceof Error) return error.message;
  if (typeof error === "string") return error;
  if (error && typeof error === "object" && "message" in error && typeof error.message === "string") {
    return error.message;
  }
  return "The operation could not be completed.";
}

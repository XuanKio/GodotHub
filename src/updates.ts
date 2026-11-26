import type { Build, Edition, InstalledEngine, Release } from "./api";

export interface EngineUpdate {
  current: InstalledEngine;
  release: Release;
  build: Build;
}

const stageOrder = { alpha: 0, dev: 1, beta: 2, rc: 3, stable: 4 } as const;
const versionPattern = /^(\d+)\.(\d+)(?:\.(\d+)(?:\.(\d+))?)?-(alpha|dev|beta|rc|stable)(\d*)$/i;

interface Version {
  parts: [major: number, minor: number, patch: number, revision: number];
  stage: number;
  sequence: number;
}

interface CurrentBuild {
  engine: InstalledEngine;
  version: Version;
}

interface ReleaseBuild {
  release: Release;
  build: Build;
  version: Version;
}

function parseVersion(tag: string): Version | undefined {
  const match = versionPattern.exec(tag.trim());
  if (!match) return undefined;

  const parts: Version["parts"] = [Number(match[1]), Number(match[2]), Number(match[3] ?? 0), Number(match[4] ?? 0)];
  const stage = stageOrder[match[5].toLowerCase() as keyof typeof stageOrder];
  const sequence = Number(match[6]);
  if (!parts.every(Number.isSafeInteger) || !Number.isSafeInteger(sequence) || (stage === stageOrder.stable && match[6])) {
    return undefined;
  }
  return { parts, stage, sequence };
}

function compareVersions(left: Version, right: Version): number {
  for (let index = 0; index < left.parts.length; index += 1) {
    const difference = left.parts[index] - right.parts[index];
    if (difference !== 0) return difference;
  }
  return left.stage - right.stage || left.sequence - right.sequence;
}

function includeNewest<T extends { version: Version }>(entries: Map<string, T>, key: string, candidate: T) {
  const previous = entries.get(key);
  if (!previous || compareVersions(candidate.version, previous.version) > 0) {
    entries.set(key, candidate);
  }
}

function tracks(version: Version, edition: Edition, arch: string) {
  const [major, minor] = version.parts;
  return { major: `${major}:${edition}:${arch}`, minor: `${major}.${minor}:${edition}:${arch}` };
}

export function updateKey(update: EngineUpdate): string {
  return `${update.release.tag}:${update.build.edition}:${update.build.arch}`;
}

export function findEngineUpdates(engines: readonly InstalledEngine[], releases: readonly Release[]): EngineUpdate[] {
  const installedStableByMajor = new Map<string, CurrentBuild>();
  const installedStableByMinor = new Map<string, CurrentBuild>();
  const installedPreviewByMinor = new Map<string, CurrentBuild>();

  for (const engine of engines) {
    const version = parseVersion(engine.tag);
    if (!version) continue;

    const { major, minor } = tracks(version, engine.edition, engine.arch);
    const installed = { engine, version };
    if (version.stage === stageOrder.stable) {
      includeNewest(installedStableByMajor, major, installed);
      includeNewest(installedStableByMinor, minor, installed);
    } else {
      includeNewest(installedPreviewByMinor, minor, installed);
    }
  }

  const releasedStableByMajor = new Map<string, ReleaseBuild>();
  const releasedByMinor = new Map<string, ReleaseBuild>();

  for (const release of releases) {
    const version = parseVersion(release.tag);
    const stable = release.channel === "stable";
    if (!version || (version.stage === stageOrder.stable) !== stable) continue;

    for (const build of release.builds) {
      const { major, minor } = tracks(version, build.edition, build.arch);
      const released = { release, build, version };
      includeNewest(releasedByMinor, minor, released);
      if (stable) includeNewest(releasedStableByMajor, major, released);
    }
  }

  const updates = new Map<string, { update: EngineUpdate; version: Version; targetVersion: Version }>();

  function includeUpdate(current: CurrentBuild, target: ReleaseBuild | undefined, stableFloor?: Version) {
    if (!target || compareVersions(target.version, current.version) <= 0) return;
    if (stableFloor && compareVersions(target.version, stableFloor) <= 0) return;

    const update = { current: current.engine, release: target.release, build: target.build };
    includeNewest(updates, updateKey(update), { update, version: current.version, targetVersion: target.version });
  }

  for (const [track, current] of installedStableByMajor) {
    includeUpdate(current, releasedStableByMajor.get(track));
  }
  for (const [track, current] of installedPreviewByMinor) {
    includeUpdate(current, releasedByMinor.get(track), installedStableByMinor.get(track)?.version);
  }

  return [...updates.values()]
    .sort((left, right) => compareVersions(right.targetVersion, left.targetVersion))
    .map(({ update }) => update);
}

export interface EditorRequest {
  major: number | null;
  minor: number | null;
  /** Tag of the editor the project was last pinned to, if any. */
  tag: string | null;
  edition: Edition;
}

export interface EditorCandidate {
  release: Release;
  build: Build;
  /** The exact editor the project used before, or the newest stable of its minor version. */
  recommended: boolean;
}

/**
 * Releases that can open a project: its previously pinned tag first, then stable releases of the
 * project's major version, preferring its minor version, newest first.
 */
export function findEditorCandidates(releases: readonly Release[], request: EditorRequest, arch: string): EditorCandidate[] {
  const candidates: { release: Release; build: Build; version: Version; exact: boolean; sameMinor: boolean }[] = [];
  for (const release of releases) {
    const version = parseVersion(release.tag);
    const build = release.builds.find((item) => item.edition === request.edition && item.arch === arch);
    if (!version || !build) continue;
    const exact = release.tag === request.tag;
    const [major, minor] = version.parts;
    if (!exact && (release.channel !== "stable" || version.stage !== stageOrder.stable)) continue;
    if (!exact && request.major !== null && major !== request.major) continue;
    if (!exact && request.major === null && major < 2) continue;
    candidates.push({ release, build, version, exact, sameMinor: request.minor !== null && minor === request.minor });
  }
  candidates.sort((left, right) => Number(right.exact) - Number(left.exact)
    || Number(right.sameMinor) - Number(left.sameMinor)
    || compareVersions(right.version, left.version));
  return candidates.map(({ release, build }, index) => ({ release, build, recommended: index === 0 }));
}

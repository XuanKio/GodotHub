import assert from "node:assert/strict";
import test from "node:test";
import type { Build, Channel, InstalledEngine, Release } from "../src/api.ts";
import { findEngineUpdates, updateKey } from "../src/updates.ts";

function build(edition: Build["edition"] = "standard", arch: Build["arch"] = "x64"): Build {
  return { edition, arch, engine: { name: "editor.zip", url: "https://example.com/editor.zip", size: 1 } };
}

function release(tag: string, channel: Channel = "stable", builds = [build()]): Release {
  return { tag, channel, notes: "", publishedAt: "", url: "", builds };
}

function installed(tag: string, fields: Partial<InstalledEngine> = {}): InstalledEngine {
  return {
    id: tag,
    tag,
    edition: "standard",
    arch: "x64",
    executable: "C:/Godot/editor.exe",
    managed: true,
    isDefault: false,
    templatesInstalled: false,
    installedAt: 1,
    ...fields,
  };
}

test("updates the highest installed stable of each major to the newest stable in that major", () => {
  const current4 = installed("4.9.2-stable");
  const current3 = installed("3.6-stable");
  const newest4 = release("4.10-stable");
  const newest3 = release("3.6.3-stable");
  const updates = findEngineUpdates([installed("4.5-stable"), current3, current4], [
    release("5.0-stable"), release("4.11-dev1", "dev"), release("4.9.10-stable"), newest3, newest4,
  ]);

  assert.deepEqual(updates, [
    { current: current4, release: newest4, build: newest4.builds[0] },
    { current: current3, release: newest3, build: newest3.builds[0] },
  ]);
});

test("updates a preview within its minor and honors an installed stable floor", () => {
  const engines = [installed("4.7-beta2"), installed("4.7-rc1"), installed("4.7-stable")];
  const lowerOrEqual = [release("4.7-rc2", "rc"), release("4.7-stable"), release("4.8-beta4", "beta")];
  assert.deepEqual(findEngineUpdates(engines, lowerOrEqual), []);

  const higher = release("4.7.1-rc1", "rc");
  const [update] = findEngineUpdates(engines, [...lowerOrEqual, higher]);
  assert.equal(update.current.tag, "4.7-rc1");
  assert.equal(update.release, higher);
});

test("orders numbered stages from alpha through stable", () => {
  const releases = [
    release("4.0-alpha20", "dev"), release("4.0-dev9", "dev"), release("4.0-dev10", "dev"),
    release("4.0-beta2", "beta"), release("4.0-rc1", "rc"), release("4.0-stable"),
  ];
  for (let index = 1; index < releases.length; index += 1) {
    const previous = releases[index - 1];
    const expected = releases[index];
    const [update] = findEngineUpdates([installed(previous.tag)], releases.slice(0, index + 1).reverse());
    assert.equal(update.release, expected);
  }
});

test("tracks builds by edition and architecture and recognizes imported versions", () => {
  const imported = installed(" 4.7.0-Stable ", { managed: false });
  const standardX64 = installed("4.6-stable");
  const standardArm = installed("4.6-stable", { arch: "arm64" });
  const dotnetX64 = installed("4.6-stable", { edition: "dotnet" });
  const newest = release("4.7-stable", "stable", [build(), build("dotnet"), build("standard", "arm64")]);
  const updates = findEngineUpdates([standardX64, standardArm, dotnetX64, imported], [release("4.6.1-stable"), newest]);

  assert.deepEqual(updates.map(updateKey).sort(), ["4.7-stable:dotnet:x64", "4.7-stable:standard:arm64"]);
});

test("deduplicates converging tracks without changing installed engines or the default", () => {
  const defaultEngine = Object.freeze(installed("4.6-stable", { isDefault: true }));
  const preview = Object.freeze(installed("4.7-rc1"));
  const engines = Object.freeze([defaultEngine, preview]);
  const target = release("4.7-stable");
  const updates = findEngineUpdates(engines, [target, target]);

  assert.deepEqual(updates, [{ current: preview, release: target, build: target.builds[0] }]);
  assert.equal(defaultEngine.isDefault, true);
  assert.equal(preview.isDefault, false);
});

test("compares a fourth legacy version component and skips unknown tags", () => {
  const current = installed("2.0.4.2-stable");
  const target = release("2.0.4.10-stable");
  const older = release("2.0.4.1-stable");
  const unsupported = release("2.1-alpha0-unofficial", "dev");
  const updates = findEngineUpdates([installed("2.1-wip"), current], [target, older, unsupported]);

  assert.deepEqual(updates, [{ current, release: target, build: target.builds[0] }]);
  assert.deepEqual(findEngineUpdates([installed("2.0.4.10-stable", { managed: false }), current], [older, target]), []);
});

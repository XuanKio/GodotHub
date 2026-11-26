import { useEffect, useMemo, useState } from "react";
import { ChevronDown, Download, LoaderCircle } from "lucide-react";
import { api, architectureLabels, channelLabels, editionLabels, errorMessage } from "../../api";
import type { Edition, Project } from "../../api";
import { ErrorNotice, Modal } from "../../components";
import { findEditorCandidates } from "../../updates";
import type { Hub } from "../../useHub";

const MAX_CHOICES = 12;

/** Suggests a Godot release that can open the project, installs it and assigns it when done. */
export function InstallEditorDialog({
  project,
  hub,
  onClose,
  onBrowse,
}: {
  project: Project;
  hub: Hub;
  onClose: () => void;
  onBrowse: () => void;
}) {
  const { catalogue, loadCatalogue } = hub;
  const arch = hub.architecture ?? "x64";
  const edition: Edition = project.engineEdition ?? (project.requiresDotnet ? "dotnet" : "standard");
  const [selected, setSelected] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  useEffect(() => {
    if (!catalogue.data && !catalogue.loading) void loadCatalogue(false);
  }, [catalogue.data, catalogue.loading, loadCatalogue]);

  const candidates = useMemo(() => findEditorCandidates(catalogue.data?.releases ?? [], {
    major: project.godotMajor,
    minor: project.godotMinor,
    tag: project.engineTag,
    edition,
  }, arch).slice(0, MAX_CHOICES), [catalogue.data, project, edition, arch]);
  const choice = candidates.find((item) => item.release.tag === selected) ?? candidates[0];

  const requirement = project.godotMajor === null
    ? "Godot"
    : `Godot ${project.godotMajor}${project.godotMinor === null ? ".x" : `.${project.godotMinor}`}`;
  const reason = project.engineTag
    ? `${project.name} was last opened with Godot ${project.engineTag}, which is no longer installed.`
    : `${project.name} needs ${requirement}${edition === "dotnet" ? " .NET" : ""}.`;

  async function install() {
    if (!choice || busy) return;
    setBusy(true);
    setError("");
    try {
      const job = await api.install(choice.release.tag, choice.build.edition, choice.build.arch);
      hub.recordJob(job);
      hub.requestEditor(project.id, { tag: choice.release.tag, edition: choice.build.edition, arch: choice.build.arch, jobId: job.id });
      hub.setNotice({ type: "success", message: `Installing Godot ${choice.release.tag} for ${project.name}.`, action: "downloads" });
      onClose();
    } catch (cause) {
      setError(errorMessage(cause));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal title={`Install editor for ${project.name}`} onClose={onClose} busy={busy}>
      <form className="project-form" onSubmit={(event) => { event.preventDefault(); void install(); }}>
        {(error || catalogue.error) && <ErrorNotice message={error || catalogue.error} retry={error ? undefined : () => void loadCatalogue(true)} />}
        <p className="modal-copy">{reason} Confirm the version to install; Hub assigns it to the project when the download finishes.</p>
        {catalogue.loading && !catalogue.data && <p className="modal-copy"><LoaderCircle size={14} className="spin" /> Loading Godot releases…</p>}
        {catalogue.data && !candidates.length && (
          <div className="project-install-hint">
            <p className="modal-copy">No {editionLabels[edition]} {architectureLabels[arch] ?? arch} release matches this project.</p>
            <button type="button" className="button secondary small" onClick={onBrowse}>Browse releases</button>
          </div>
        )}
        {choice && (
          <label className="project-field">
            <span>Godot version</span>
            <span className="select-field">
              <select value={choice.release.tag} onChange={(event) => setSelected(event.target.value)} disabled={busy}>
                {candidates.map(({ release, recommended }) => (
                  <option key={release.tag} value={release.tag}>
                    Godot {release.tag} · {channelLabels[release.channel]}{recommended ? " (suggested)" : ""}
                  </option>
                ))}
              </select>
              <ChevronDown size={14} aria-hidden="true" />
            </span>
            <span className="modal-copy">{editionLabels[choice.build.edition]} · {architectureLabels[choice.build.arch] ?? choice.build.arch}</span>
          </label>
        )}
        <div className="modal-actions">
          <button type="button" className="button secondary" onClick={onClose} disabled={busy}>Cancel</button>
          <button type="submit" className="button primary" disabled={!choice || busy}>
            {busy ? <LoaderCircle size={16} className="spin" /> : <Download size={16} />} Install {choice ? `Godot ${choice.release.tag}` : "editor"}
          </button>
        </div>
      </form>
    </Modal>
  );
}

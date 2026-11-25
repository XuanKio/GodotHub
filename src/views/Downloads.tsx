import { ArrowDownToLine, Check, CircleAlert, DownloadCloud, Package, Timer } from "lucide-react";
import { formatDate, formatSize, isActive } from "../api";
import type { Job, Stage } from "../api";
import { ErrorNotice, LoadingState, StatePanel } from "../components";
import type { Hub } from "../useHub";

const stageLabels: Record<Stage, string> = {
  queued: "Queued",
  downloading: "Downloading",
  verifying: "Verifying checksum",
  extracting: "Extracting",
  installing: "Installing",
  completed: "Completed",
  failed: "Failed",
};

function DownloadRow({ job }: { job: Job }) {
  const active = isActive(job);
  const downloading = job.stage === "downloading";
  const showPercentage = downloading && job.total > 0;
  const percent = showPercentage ? Math.min(100, Math.floor(job.downloaded / job.total * 100)) : undefined;
  const Icon = job.stage === "completed" ? Check : job.stage === "failed" ? CircleAlert :
    job.stage === "queued" ? Timer : job.kind === "templates" ? Package : ArrowDownToLine;

  return (
    <li className={`item-row download-row job-${job.stage}`}>
      <span className="version-icon job-icon"><Icon size={22} aria-hidden="true" /></span>
      <div className="item-content">
        <div className="item-title">
          <h2>{job.label}</h2>
        </div>
        <div className="item-meta">
          <span>{job.kind === "templates" ? "Export templates" : "Engine"}</span>
          {formatDate(job.startedAt) && <span>{formatDate(job.startedAt)}</span>}
        </div>
        {active && (
          <div className="job-progress">
            {job.stage !== "queued" && (
              <progress aria-label={`${stageLabels[job.stage]} ${job.label}`} max={100} value={percent} />
            )}
            {downloading && (
              <span>{formatSize(job.downloaded)}{job.total > 0 && ` / ${formatSize(job.total)}`}</span>
            )}
          </div>
        )}
        {job.error && <p className="job-error">{job.error}</p>}
      </div>
      <span className="job-status">
        {stageLabels[job.stage]}{percent !== undefined ? ` · ${percent}%` : ""}
      </span>
    </li>
  );
}

export function Downloads({ hub, onBrowse }: { hub: Hub; onBrowse: () => void }) {
  const active = hub.jobs.filter(isActive);
  const history = hub.jobs.filter((job) => !isActive(job));

  return (
    <>
      <header className="page-heading">
        <div>
          <h1>Downloads</h1>
          <p>Track engine and export template installations.</p>
        </div>
        {hub.jobs.length > 0 && (
          <button type="button" className="button secondary" onClick={onBrowse}>
            <ArrowDownToLine size={17} /> Browse releases
          </button>
        )}
      </header>
      <div className="view-section downloads-section">
        {hub.jobsError && <ErrorNotice message={hub.jobsError} retry={() => void hub.loadJobs()} />}
        {hub.jobsLoading && !hub.jobs.length && <LoadingState title="Loading downloads" />}
        {!hub.jobsLoading && !hub.jobsError && !hub.jobs.length && (
          <StatePanel icon={DownloadCloud} title="No downloads yet" description="New installs and their progress will appear here.">
            <button type="button" className="button primary" onClick={onBrowse}><ArrowDownToLine size={17} /> Browse releases</button>
          </StatePanel>
        )}
        {active.length > 0 && (
          <section className="download-group" aria-label="Downloads in progress">
            <h2 className="section-title">In progress <span className="tab-count">{active.length}</span></h2>
            <ul className="item-list">{active.map((job) => <DownloadRow key={job.id} job={job} />)}</ul>
          </section>
        )}
        {history.length > 0 && (
          <section className="download-group" aria-label="Download history">
            <h2 className="section-title">This session <span className="tab-count">{history.length}</span></h2>
            <ul className="item-list">{history.map((job) => <DownloadRow key={job.id} job={job} />)}</ul>
          </section>
        )}
      </div>
    </>
  );
}

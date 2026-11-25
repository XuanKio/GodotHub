import { useId, useLayoutEffect, useRef } from "react";
import type { ReactNode } from "react";
import { AlertCircle, LoaderCircle, X } from "lucide-react";
import type { LucideIcon } from "lucide-react";

interface StatePanelProps {
  icon: LucideIcon;
  title: string;
  description?: string;
  children?: ReactNode;
  loading?: boolean;
}

export function StatePanel({
  icon: Icon,
  title,
  description,
  children,
  loading,
}: StatePanelProps) {
  return (
    <div className="state-panel" role={loading ? "status" : undefined}>
      <span className="state-icon">
        <Icon size={27} className={loading ? "spin" : undefined} aria-hidden="true" />
      </span>
      <h2>{title}</h2>
      {description && <p>{description}</p>}
      {children && <div className="state-actions">{children}</div>}
    </div>
  );
}

export function LoadingState({ title }: { title: string }) {
  return <StatePanel icon={LoaderCircle} title={title} loading />;
}

export function ErrorNotice({
  message,
  retry,
}: {
  message: string;
  retry?: () => void;
}) {
  return (
    <div className="notice error-notice" role="alert">
      <AlertCircle size={18} aria-hidden="true" />
      <p>{message}</p>
      {retry && <button type="button" className="button secondary small" onClick={retry}>Retry</button>}
    </div>
  );
}

export function Modal({
  title,
  children,
  onClose,
  busy = false,
}: {
  title: string;
  children: ReactNode;
  onClose: () => void;
  busy?: boolean;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const titleId = useId();

  useLayoutEffect(() => {
    const element = dialog.current;
    element?.showModal();
    return () => element?.close();
  }, []);

  return (
    <dialog
      ref={dialog}
      className="modal"
      aria-labelledby={titleId}
      onCancel={(event) => {
        event.preventDefault();
        if (!busy) onClose();
      }}
    >
      <header className="modal-header">
        <h2 id={titleId}>{title}</h2>
        <button type="button" className="button icon-button ghost" aria-label="Close dialog" disabled={busy} onClick={onClose}>
          <X size={18} aria-hidden="true" />
        </button>
      </header>
      <div className="modal-body">{children}</div>
    </dialog>
  );
}

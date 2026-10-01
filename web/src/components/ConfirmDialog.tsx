import { useEffect, useRef } from "react";
import { Trash2 } from "lucide-react";

export function ConfirmDialog({ title, description, busy, error, onCancel, onConfirm }: { title: string; description: string; busy: boolean; error: string | null; onCancel: () => void; onConfirm: () => void }) {
  const ref = useRef<HTMLElement>(null);
  const cancel = useRef<HTMLButtonElement>(null);
  const latest = useRef({ busy, onCancel });
  latest.current = { busy, onCancel };
  useEffect(() => {
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    cancel.current?.focus();
    const key = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !latest.current.busy) latest.current.onCancel();
      if (event.key !== "Tab") return;
      const buttons = Array.from(ref.current?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)") ?? []);
      if (!buttons.length) { event.preventDefault(); ref.current?.focus(); }
      else if (event.shiftKey && document.activeElement === buttons[0]) { event.preventDefault(); buttons.at(-1)?.focus(); }
      else if (!event.shiftKey && document.activeElement === buttons.at(-1)) { event.preventDefault(); buttons[0]?.focus(); }
    };
    document.addEventListener("keydown", key);
    return () => { document.removeEventListener("keydown", key); previous?.focus(); };
  }, []);
  return <div className="confirm-layer"><button type="button" className="confirm-backdrop" tabIndex={-1} aria-label="关闭确认" onClick={onCancel} disabled={busy} /><section ref={ref} className="confirm-dialog" role="alertdialog" aria-modal="true" aria-labelledby="confirm-title" aria-describedby="confirm-description" tabIndex={-1}><h2 id="confirm-title">{title}</h2><p id="confirm-description">{description}</p>{error && <p className="form-error" role="alert">{error}</p>}<footer><button ref={cancel} type="button" className="secondary-button" onClick={onCancel} disabled={busy}>取消</button><button type="button" className="danger-button" onClick={onConfirm} disabled={busy}><Trash2 size={16} />{busy ? "正在删除" : "确认删除"}</button></footer></section></div>;
}

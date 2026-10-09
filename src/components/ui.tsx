import { createContext, ReactNode, useCallback, useContext, useEffect, useState } from "react";
import { X, Loader2 } from "lucide-react";

export function cx(...c: (string | false | null | undefined)[]) {
  return c.filter(Boolean).join(" ");
}

type Variant = "primary" | "secondary" | "ghost" | "danger" | "rec";

export function Button({
  children, onClick, variant = "secondary", disabled, icon, size = "md", title, type = "button", className, loading,
}: {
  children?: ReactNode; onClick?: () => void; variant?: Variant; disabled?: boolean; icon?: ReactNode;
  size?: "sm" | "md" | "lg"; title?: string; type?: "button" | "submit"; className?: string; loading?: boolean;
}) {
  const styles: Record<Variant, string> = {
    primary: "bg-accent text-white hover:bg-accent-hover shadow-sm",
    secondary: "bg-panel border border-line text-fg hover:bg-panel-2 shadow-sm",
    ghost: "text-muted hover:text-fg hover:bg-panel-2",
    danger: "bg-panel border border-line text-red-500 hover:bg-red-500/10",
    rec: "bg-rec text-white hover:brightness-110 shadow-sm",
  };
  const sizes = { sm: "h-7 px-2.5 text-[12.5px] gap-1.5", md: "h-8 px-3 gap-2", lg: "h-10 px-4 text-[14px] gap-2" };
  return (
    <button
      type={type}
      title={title}
      disabled={disabled || loading}
      onClick={onClick}
      className={cx(
        "inline-flex items-center justify-center rounded-md font-medium transition-colors whitespace-nowrap",
        "disabled:opacity-45 disabled:cursor-not-allowed focus-visible:outline-2 focus-visible:outline-accent",
        styles[variant], sizes[size], className,
      )}
    >
      {loading ? <Loader2 size={14} className="animate-spin" /> : icon}
      {children}
    </button>
  );
}

export function Card({ children, className, title, actions }: { children: ReactNode; className?: string; title?: ReactNode; actions?: ReactNode }) {
  return (
    <section className={cx("bg-panel border border-line rounded-xl", className)}>
      {(title || actions) && (
        <header className="flex items-center justify-between px-4 pt-3.5 pb-2">
          <h3 className="text-[12px] font-semibold uppercase tracking-wide text-muted">{title}</h3>
          <div className="flex gap-2">{actions}</div>
        </header>
      )}
      <div className={title || actions ? "px-4 pb-4" : "p-4"}>{children}</div>
    </section>
  );
}

export function Badge({ children, tone = "neutral" }: { children: ReactNode; tone?: "neutral" | "green" | "yellow" | "red" | "blue" | "accent" }) {
  const tones = {
    neutral: "bg-panel-2 text-muted border-line",
    green: "bg-emerald-500/10 text-emerald-600 dark:text-emerald-400 border-emerald-500/20",
    yellow: "bg-amber-500/10 text-amber-600 dark:text-amber-400 border-amber-500/20",
    red: "bg-red-500/10 text-red-600 dark:text-red-400 border-red-500/20",
    blue: "bg-sky-500/10 text-sky-600 dark:text-sky-400 border-sky-500/20",
    accent: "bg-accent/10 text-accent border-accent/20",
  };
  return <span className={cx("inline-flex items-center gap-1 h-5 px-1.5 rounded border text-[11px] font-medium", tones[tone])}>{children}</span>;
}

export function Field({ label, hint, children }: { label: string; hint?: ReactNode; children: ReactNode }) {
  return (
    <label className="block">
      <div className="text-[12px] font-medium text-muted mb-1.5">{label}</div>
      {children}
      {hint && <div className="text-[11.5px] text-subtle mt-1">{hint}</div>}
    </label>
  );
}

export const inputCls =
  "w-full h-8 px-2.5 rounded-md bg-panel border border-line text-fg placeholder:text-subtle outline-none focus:border-accent focus:ring-2 focus:ring-accent/20 transition";

export function Input(props: React.InputHTMLAttributes<HTMLInputElement>) {
  return <input {...props} className={cx(inputCls, props.className)} />;
}

export function Select<T extends string>({ value, onChange, options, disabled, className }: {
  value: T; onChange: (v: T) => void; options: { value: T; label: string }[]; disabled?: boolean; className?: string;
}) {
  return (
    <select value={value} disabled={disabled} onChange={(e) => onChange(e.target.value as T)} className={cx(inputCls, "pr-7", className)}>
      {options.map((o) => (
        <option key={o.value} value={o.value}>{o.label}</option>
      ))}
    </select>
  );
}

export function Toggle({ checked, onChange, label, description, disabled }: {
  checked: boolean; onChange: (v: boolean) => void; label: ReactNode; description?: ReactNode; disabled?: boolean;
}) {
  return (
    <button
      type="button"
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className="flex items-start gap-3 text-left w-full group disabled:opacity-50"
    >
      <span className={cx("mt-0.5 relative inline-flex h-[18px] w-8 shrink-0 rounded-full transition-colors", checked ? "bg-accent" : "bg-zinc-300 dark:bg-zinc-700")}>
        <span className={cx("absolute top-[2px] h-[14px] w-[14px] rounded-full bg-white shadow transition-transform", checked ? "translate-x-[16px]" : "translate-x-[2px]")} />
      </span>
      <span>
        <span className="block text-fg font-medium">{label}</span>
        {description && <span className="block text-[12px] text-muted mt-0.5">{description}</span>}
      </span>
    </button>
  );
}

export function Segmented<T extends string>({ value, onChange, options }: { value: T; onChange: (v: T) => void; options: { value: T; label: ReactNode }[] }) {
  return (
    <div className="inline-flex p-0.5 rounded-lg bg-panel-2 border border-line">
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          onClick={() => onChange(o.value)}
          className={cx(
            "h-7 px-3 rounded-md text-[12.5px] font-medium transition flex items-center gap-1.5",
            value === o.value ? "bg-panel text-fg shadow-sm" : "text-muted hover:text-fg",
          )}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

/** Horizontal audio level meter (dBFS). */
export function LevelMeter({ db, className }: { db: number; className?: string }) {
  const pct = Math.max(0, Math.min(100, ((db + 60) / 60) * 100));
  return (
    <div className={cx("h-2 rounded-full bg-panel-2 border border-line overflow-hidden", className)}>
      <div
        className={cx("h-full transition-[width] duration-150", db > -6 ? "bg-red-500" : db > -20 ? "bg-amber-400" : "bg-emerald-500")}
        style={{ width: `${pct}%` }}
      />
    </div>
  );
}

export function Progress({ value, className }: { value: number; className?: string }) {
  return (
    <div className={cx("h-1.5 rounded-full bg-panel-2 overflow-hidden", className)}>
      <div className="h-full bg-accent transition-[width] duration-300" style={{ width: `${Math.max(0, Math.min(1, value)) * 100}%` }} />
    </div>
  );
}

export function Modal({ open, onClose, title, children, footer, width = 520 }: {
  open: boolean; onClose: () => void; title: ReactNode; children: ReactNode; footer?: ReactNode; width?: number;
}) {
  useEffect(() => {
    if (!open) return;
    const h = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", h);
    return () => window.removeEventListener("keydown", h);
  }, [open, onClose]);
  if (!open) return null;
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 backdrop-blur-[2px] p-6" onMouseDown={onClose}>
      <div
        className="bg-panel border border-line rounded-xl shadow-2xl max-h-full flex flex-col"
        style={{ width }}
        onMouseDown={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between px-5 pt-4 pb-2">
          <h2 className="text-[15px] font-semibold">{title}</h2>
          <button onClick={onClose} className="text-muted hover:text-fg p-1 rounded"><X size={16} /></button>
        </div>
        <div className="px-5 pb-4 overflow-auto">{children}</div>
        {footer && <div className="px-5 py-3 border-t border-line flex justify-end gap-2">{footer}</div>}
      </div>
    </div>
  );
}

export function Empty({ icon, title, children }: { icon: ReactNode; title: string; children?: ReactNode }) {
  return (
    <div className="flex flex-col items-center justify-center text-center py-14 px-6">
      <div className="text-subtle mb-3">{icon}</div>
      <div className="font-semibold text-fg">{title}</div>
      {children && <div className="text-muted text-[12.5px] mt-1 max-w-sm">{children}</div>}
    </div>
  );
}

// ---- toasts --------------------------------------------------------------------
type Toast = { id: number; text: string; tone: "info" | "error" | "success" };
const ToastCtx = createContext<(text: string, tone?: Toast["tone"]) => void>(() => {});
export const useToast = () => useContext(ToastCtx);

export function ToastProvider({ children }: { children: ReactNode }) {
  const [toasts, setToasts] = useState<Toast[]>([]);
  const push = useCallback((text: string, tone: Toast["tone"] = "info") => {
    const id = Date.now() + Math.random();
    setToasts((t) => [...t, { id, text, tone }]);
    setTimeout(() => setToasts((t) => t.filter((x) => x.id !== id)), tone === "error" ? 8000 : 3500);
  }, []);
  return (
    <ToastCtx.Provider value={push}>
      {children}
      <div className="fixed bottom-4 right-4 z-[60] flex flex-col gap-2 max-w-sm">
        {toasts.map((t) => (
          <div
            key={t.id}
            className={cx(
              "px-3.5 py-2.5 rounded-lg shadow-lg border text-[13px] bg-panel border-line selectable",
              t.tone === "error" && "border-red-500/40 text-red-600 dark:text-red-400",
              t.tone === "success" && "border-emerald-500/40",
            )}
          >
            {t.text}
          </div>
        ))}
      </div>
    </ToastCtx.Provider>
  );
}

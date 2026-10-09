export function fmtMs(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  const mm = String(m).padStart(2, "0");
  const ss = String(sec).padStart(2, "0");
  return h > 0 ? `${h}:${mm}:${ss}` : `${mm}:${ss}`;
}

export function fmtDuration(ms: number): string {
  const min = Math.round(ms / 60000);
  if (min < 1) return `${Math.round(ms / 1000)} s`;
  const h = Math.floor(min / 60);
  const m = min % 60;
  return h > 0 ? `${h} h ${String(m).padStart(2, "0")} min` : `${m} min`;
}

export function fmtBytes(b: number | null | undefined): string {
  if (b === null || b === undefined) return "—";
  const u = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  let v = b;
  while (v >= 1024 && i < u.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v.toFixed(v >= 100 || i === 0 ? 0 : 1)} ${u[i]}`;
}

export function fmtDate(iso: string): string {
  const d = new Date(iso);
  return d.toLocaleDateString("pl-PL", { day: "numeric", month: "long", year: "numeric" });
}

export function fmtDateTime(iso: string): string {
  const d = new Date(iso);
  return d.toLocaleString("pl-PL", { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" });
}

export function parseTime(text: string): number | null {
  const parts = text.trim().split(":").map((p) => Number(p));
  if (parts.some((p) => Number.isNaN(p) || p < 0)) return null;
  let s = 0;
  for (const p of parts) s = s * 60 + p;
  return Math.round(s * 1000);
}

export const transcriptionLabel: Record<string, string> = {
  disabled: "wyłączona",
  pending: "oczekuje",
  running: "w toku",
  completed: "gotowa",
  partial: "częściowa",
  failed: "błąd",
};

export function errorText(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return JSON.stringify(e);
}

/** Polish plural: plural(3, "slajd", "slajdy", "slajdów") → "3 slajdy". */
export function plural(n: number, one: string, few: string, many: string): string {
  const mod10 = n % 10;
  const mod100 = n % 100;
  const w = n === 1 ? one : mod10 >= 2 && mod10 <= 4 && (mod100 < 12 || mod100 > 14) ? few : many;
  return `${n} ${w}`;
}

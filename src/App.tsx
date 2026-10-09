import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { BookOpen, Plus, Settings as SettingsIcon, Radio, Cpu } from "lucide-react";
import { api, AutoStopped, Settings } from "./lib/api";
import { cx, ToastProvider, useToast } from "./components/ui";
import { Dashboard } from "./views/Dashboard";
import { NewLecture } from "./views/NewLecture";
import { Recording } from "./views/Recording";
import { LectureDetailView } from "./views/LectureDetail";
import { SettingsView } from "./views/Settings";
import { ModelsView } from "./views/Models";
import { Consent } from "./views/Consent";

export type Route =
  | { name: "dashboard" }
  /** `subject`: preselected subject ("" = none, undefined = last used). */
  | { name: "new"; subject?: string }
  | { name: "recording" }
  | { name: "lecture"; path: string }
  | { name: "models" }
  | { name: "settings" };

export interface Nav {
  go: (r: Route) => void;
  settings: Settings;
  reloadSettings: () => Promise<void>;
  updateSettings: (s: Settings) => Promise<void>;
}

function applyTheme(theme: Settings["theme"]) {
  const dark = theme === "dark" || (theme === "system" && window.matchMedia("(prefers-color-scheme: dark)").matches);
  document.documentElement.classList.toggle("dark", dark);
}

function Shell() {
  const [route, setRoute] = useState<Route>((window as unknown as { __LC_INITIAL_ROUTE__?: Route }).__LC_INITIAL_ROUTE__ ?? { name: "dashboard" });
  const [settings, setSettings] = useState<Settings | null>(null);
  const [recording, setRecording] = useState(false);
  const toast = useToast();

  const reloadSettings = useCallback(async () => setSettings(await api.getSettings()), []);
  const updateSettings = useCallback(async (s: Settings) => {
    await api.saveSettings(s);
    setSettings(s);
  }, []);

  useEffect(() => {
    reloadSettings();
    api.recordingStatus().then((s) => {
      if (s) {
        setRecording(true);
        setRoute({ name: "recording" });
      }
    });
    api.takeRecoveries().then((reports) => {
      for (const r of reports) {
        toast(`Odzyskano przerwany wykład „${r.folder}”. ${r.pending_chunks ? "Dokańczam transkrypcję w tle." : ""}`, "success");
      }
    });
  }, [reloadSettings, toast]);

  useEffect(() => {
    if (!settings) return;
    applyTheme(settings.theme);
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const h = () => applyTheme(settings.theme);
    mq.addEventListener("change", h);
    return () => mq.removeEventListener("change", h);
  }, [settings]);

  // recording state drives the window title and navigation guard
  useEffect(() => {
    const t = setInterval(async () => {
      const s = await api.recordingStatus();
      setRecording(!!s);
    }, 1500);
    return () => clearInterval(t);
  }, []);

  useEffect(() => {
    getCurrentWindow().setTitle(recording ? "● Nagrywanie — LectureCapture" : "LectureCapture").catch(() => {});
  }, [recording]);

  useEffect(() => {
    const un1 = listen("close-requested", () => {
      setRoute({ name: "recording" });
      toast("Trwa nagrywanie. Zatrzymaj je przyciskiem „Zatrzymaj i zapisz”, zanim zamkniesz aplikację.", "error");
    });
    const un2 = listen("stop-requested", () => setRoute({ name: "recording" }));
    const un3 = listen("auto-stop-started", () => toast("Ustawiona godzina końca – zapisuję wykład…", "info"));
    const un4 = listen<AutoStopped>("auto-stopped", (e) => {
      const r = e.payload;
      setRecording(false);
      if (r.error) toast(`Automatyczne zakończenie nie powiodło się: ${r.error}`, "error");
      else toast(`Wykład zapisany automatycznie${r.left_meeting ? " i opuszczono spotkanie Teams" : ""}.`, "success");
      if (r.leave_error) toast(`Nie udało się opuścić spotkania: ${r.leave_error}`, "error");
      if (r.path) setRoute({ name: "lecture", path: r.path });
    });
    return () => {
      un1.then((f) => f());
      un2.then((f) => f());
      un3.then((f) => f());
      un4.then((f) => f());
    };
  }, [toast]);

  if (!settings) return <div className="h-full bg-app" />;

  const nav: Nav = { go: setRoute, settings, reloadSettings, updateSettings };
  const items = [
    { id: "dashboard", label: "Wykłady", icon: <BookOpen size={15} />, route: { name: "dashboard" } as Route },
    recording
      ? { id: "recording", label: "Nagrywanie", icon: <Radio size={15} className="text-rec" />, route: { name: "recording" } as Route }
      : { id: "new", label: "Nowy wykład", icon: <Plus size={15} />, route: { name: "new" } as Route },
    { id: "models", label: "Modele Whisper", icon: <Cpu size={15} />, route: { name: "models" } as Route },
    { id: "settings", label: "Ustawienia", icon: <SettingsIcon size={15} />, route: { name: "settings" } as Route },
  ];

  return (
    <div className="h-full flex bg-app text-fg">
      <aside className="w-[214px] shrink-0 border-r border-line bg-panel/60 flex flex-col">
        <div data-tauri-drag-region className="h-[52px] shrink-0" />
        <div className="px-4 pb-4 flex items-center gap-2.5" data-tauri-drag-region>
          <img src="/icon.png" alt="" className="w-7 h-7 rounded-md" />
          <div>
            <div className="font-semibold leading-tight">LectureCapture</div>
            <div className="text-[11px] text-subtle leading-tight">lokalnie · offline</div>
          </div>
        </div>
        <nav className="px-2 flex flex-col gap-0.5">
          {items.map((it) => (
            <button
              key={it.id}
              onClick={() => setRoute(it.route)}
              className={cx(
                "flex items-center gap-2.5 h-8 px-2.5 rounded-md text-left font-medium transition-colors",
                route.name === it.id ? "bg-panel-2 text-fg" : "text-muted hover:text-fg hover:bg-panel-2/60",
              )}
            >
              {it.icon}
              {it.label}
              {it.id === "recording" && <span className="ml-auto w-2 h-2 rounded-full bg-rec rec-dot" />}
            </button>
          ))}
        </nav>
        <div className="mt-auto p-4 text-[11px] text-subtle leading-relaxed">
          Wszystko zostaje na tym komputerze. Brak telemetrii, kont i serwerów.
        </div>
      </aside>
      <main className="flex-1 min-w-0 flex flex-col">
        <div data-tauri-drag-region className="h-[38px] shrink-0" />
        <div className="flex-1 min-h-0 overflow-auto">
          {route.name === "dashboard" && <Dashboard nav={nav} recording={recording} />}
          {route.name === "new" && (recording ? <Recording nav={nav} /> : <NewLecture nav={nav} subject={route.subject} onStarted={() => { setRecording(true); setRoute({ name: "recording" }); }} />)}
          {route.name === "recording" && <Recording nav={nav} />}
          {route.name === "lecture" && <LectureDetailView nav={nav} path={route.path} />}
          {route.name === "models" && <ModelsView />}
          {route.name === "settings" && <SettingsView nav={nav} />}
        </div>
      </main>
      {!settings.consent_acknowledged && <Consent nav={nav} />}
    </div>
  );
}

export default function App() {
  return (
    <ToastProvider>
      <Shell />
    </ToastProvider>
  );
}

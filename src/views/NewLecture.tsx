import { useCallback, useEffect, useMemo, useState } from "react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { AppWindow, Monitor, RefreshCw, FolderOpen, Volume2, Mic, Play, Crop, Info, Download, Clock, LogOut, AlertTriangle } from "lucide-react";
import {
  api, AudioDevices, AudioSelection, AudioTestSource, AutoStopInfo, CaptureTarget, LANGUAGES, NormRect, SourceList, SubjectInfo,
} from "../lib/api";
import { errorText, fmtBytes } from "../lib/format";
import { Badge, Button, Card, Field, Input, LevelMeter, Segmented, Select, Toggle, useToast, cx } from "../components/ui";
import { CropSelector } from "../components/CropSelector";
import { PermissionRow } from "./Consent";
import { useModels } from "./Models";
import type { Nav } from "../App";

const NEW_SUBJECT = "\u0000new";

/** Scheduled end + "leave the Teams meeting" options (shared with the recording view). */
export function AutoStopFields({ enabled, setEnabled, time, setTime, leave, setLeave, info }: {
  enabled: boolean; setEnabled: (v: boolean) => void; time: string; setTime: (v: string) => void;
  leave: boolean; setLeave: (v: boolean) => void; info: AutoStopInfo | null;
}) {
  return (
    <div className="space-y-2.5">
      <div className="flex items-center gap-3">
        <div className="flex-1">
          <Toggle
            checked={enabled}
            onChange={setEnabled}
            label={<span className="flex items-center gap-1.5"><Clock size={14} /> Zakończ o godzinie</span>}
            description="Nagranie zostanie zatrzymane i zapisane automatycznie."
          />
        </div>
        <input
          type="time"
          value={time}
          disabled={!enabled}
          onChange={(e) => setTime(e.target.value)}
          className="h-8 px-2 rounded-md bg-panel border border-line text-fg tabular-nums outline-none focus:border-accent disabled:opacity-45"
        />
      </div>
      {enabled && (
        <div className="pl-11 space-y-2">
          <Toggle
            checked={leave}
            onChange={setLeave}
            label={<span className="flex items-center gap-1.5"><LogOut size={14} /> Opuść spotkanie Teams</span>}
            description={`Po zapisaniu aplikacja przełączy się na okno spotkania i naciśnie ${info?.shortcut ?? "skrót"} („Opuść”).`}
          />
          {leave && info && !info.can_send_keys && (
            <div className="flex items-start gap-2 rounded-lg bg-amber-500/5 border border-amber-500/30 p-2.5 text-[12px]">
              <AlertTriangle size={14} className="text-amber-500 shrink-0 mt-0.5" />
              <div className="flex-1">
                Aby wysłać skrót do Teams, LectureCapture potrzebuje uprawnienia <b>Dostępność</b> (Ustawienia systemowe → Prywatność i ochrona → Dostępność).
                <div className="mt-1.5"><Button size="sm" onClick={() => api.openKeyPermissionSettings()}>Otwórz ustawienia</Button></div>
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

function defaultEndTime(): string {
  const d = new Date(Date.now() + 90 * 60_000);
  d.setMinutes(Math.ceil(d.getMinutes() / 15) * 15, 0, 0);
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

export function NewLecture({ nav, onStarted, subject: initialSubject }: { nav: Nav; onStarted: () => void; subject?: string }) {
  const toast = useToast();
  const s = nav.settings;
  const [title, setTitle] = useState("");
  const [subjects, setSubjects] = useState<SubjectInfo[]>([]);
  const [subject, setSubject] = useState<string>(initialSubject ?? s.last_subject ?? "");
  const [newSubject, setNewSubject] = useState("");
  const [stopEnabled, setStopEnabled] = useState(false);
  const [stopAt, setStopAt] = useState(defaultEndTime);
  const [leave, setLeave] = useState(s.auto_leave_meeting);
  const [autoInfo, setAutoInfo] = useState<AutoStopInfo | null>(null);
  const [outputDir, setOutputDir] = useState(s.lectures_root);
  const [free, setFree] = useState<number | null>(null);
  const [platform, setPlatform] = useState("macos");
  const [sources, setSources] = useState<SourceList | null>(null);
  const [sourceError, setSourceError] = useState<string | null>(null);
  const [kind, setKind] = useState<"window" | "display">(s.last_target?.kind ?? "window");
  const [windowId, setWindowId] = useState<number | null>(s.last_target?.kind === "window" ? s.last_target.id : null);
  const [displayId, setDisplayId] = useState<number | null>(s.last_target?.kind === "display" ? s.last_target.id : null);
  const [preview, setPreview] = useState<{ data_url: string; width: number; height: number } | null>(null);
  const [previewLoading, setPreviewLoading] = useState(false);
  const [crop, setCrop] = useState<NormRect | null>(s.last_crop);
  const [devices, setDevices] = useState<AudioDevices | null>(null);
  const [audio, setAudio] = useState<AudioSelection>({
    capture_system: s.audio.capture_system,
    capture_microphone: s.audio.capture_microphone,
    microphone_device: s.audio.microphone_device,
    loopback_device: s.audio.loopback_device,
    only_application: null,
  });
  const [testing, setTesting] = useState(false);
  const [test, setTest] = useState<AudioTestSource[] | null>(null);
  const [transcribe, setTranscribe] = useState(s.transcription.enabled);
  const [model, setModel] = useState(s.transcription.model);
  const [language, setLanguage] = useState(s.transcription.language);
  const [live, setLive] = useState(s.transcription.live);
  const [starting, setStarting] = useState(false);
  const { models, downloads } = useModels();

  const loadSources = useCallback(async () => {
    try {
      setSourceError(null);
      const list = await api.listSources();
      setSources(list);
      if (windowId === null || !list.windows.some((w) => w.id === windowId)) {
        const teams = list.windows.find((w) => /teams/i.test(w.app_name) || /teams/i.test(w.bundle_id));
        setWindowId(teams?.id ?? list.windows[0]?.id ?? null);
      }
      if (displayId === null) setDisplayId(list.displays[0]?.id ?? null);
    } catch (e) {
      setSourceError(errorText(e));
    }
  }, [windowId, displayId]);

  useEffect(() => {
    loadSources();
    api.listAudioDevices().then(setDevices);
    api.systemInfo().then((i) => setPlatform(i.platform));
    api.listSubjects().then(setSubjects).catch(() => {});
    api.getAutoStop().then(setAutoInfo).catch(() => {});
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    api.diskFree(outputDir).then(setFree);
  }, [outputDir]);

  const target: CaptureTarget | null = useMemo(() => {
    if (kind === "display") return displayId !== null ? { kind: "display", id: displayId } : null;
    const w = sources?.windows.find((w) => w.id === windowId);
    return w ? { kind: "window", id: w.id, bundle_id: w.bundle_id, title: w.title } : null;
  }, [kind, displayId, windowId, sources]);

  const refreshPreview = useCallback(async () => {
    if (!target) return;
    setPreviewLoading(true);
    try {
      setPreview(await api.snapshot(target));
    } catch (e) {
      setPreview(null);
      toast(errorText(e), "error");
    } finally {
      setPreviewLoading(false);
    }
  }, [target, toast]);

  useEffect(() => {
    refreshPreview();
  }, [refreshPreview]);

  const apps = useMemo(() => {
    const m = new Map<string, string>();
    sources?.windows.forEach((w) => m.set(w.bundle_id, w.app_name));
    return [...m.entries()];
  }, [sources]);

  const selectedModel = models.find((m) => m.info.id === model);
  const modelMissing = transcribe && !!selectedModel && !selectedModel.installed;
  const nothingToRecord = !target;

  const runTest = async () => {
    setTesting(true);
    setTest(null);
    try {
      setTest(await api.audioTest(audio, 3));
    } catch (e) {
      toast(errorText(e), "error");
    } finally {
      setTesting(false);
    }
  };

  // remembered subject may have been removed meanwhile
  useEffect(() => {
    if (subject && subject !== NEW_SUBJECT && subjects.length && !subjects.some((x) => x.name === subject) && initialSubject === undefined) setSubject("");
  }, [subjects]); // eslint-disable-line react-hooks/exhaustive-deps

  const subjectName = subject === NEW_SUBJECT ? newSubject.trim() : subject;
  const sep = outputDir.includes("\\") ? "\\" : "/";
  const savePath = subjectName ? `${outputDir.replace(/[\\/]+$/, "")}${sep}${subjectName}` : outputDir;

  const start = async () => {
    if (!target) return;
    if (subject === NEW_SUBJECT && !subjectName) {
      toast("Podaj nazwę nowego przedmiotu.", "error");
      return;
    }
    setStarting(true);
    try {
      await api.startRecording({
        title: title.trim() || subjectName || "Wykład",
        output_dir: outputDir,
        subject: subjectName || null,
        stop_at: stopEnabled ? stopAt : null,
        leave_meeting: leave,
        target,
        crop,
        audio,
        transcription: transcribe,
        model,
        language,
        live,
      });
      await nav.reloadSettings();
      onStarted();
    } catch (e) {
      toast(errorText(e), "error");
    } finally {
      setStarting(false);
    }
  };

  return (
    <div className="max-w-[1180px] mx-auto px-8 pb-28">
      <h1 className="text-[22px] font-semibold tracking-tight">Nowy wykład</h1>
      <p className="text-muted mt-0.5 mb-5">Wybierz okno Teams (najlepiej sam obszar prezentacji) i źródło dźwięku. Resztą zajmie się aplikacja.</p>
      <div className="mb-4"><PermissionRow compact /></div>

      <div className="grid grid-cols-[minmax(0,1fr)_400px] gap-5">
        <div className="space-y-4 min-w-0">
          <Card title="Obraz">
            <div className="flex items-center justify-between mb-3">
              <Segmented
                value={kind}
                onChange={setKind}
                options={[
                  { value: "window", label: <><AppWindow size={13} /> Okno</> },
                  { value: "display", label: <><Monitor size={13} /> Monitor</> },
                ]}
              />
              <Button size="sm" variant="ghost" icon={<RefreshCw size={13} />} onClick={() => { loadSources(); refreshPreview(); }}>Odśwież</Button>
            </div>
            {sourceError && <div className="text-red-500 text-[12.5px] mb-3 selectable">{sourceError}</div>}
            {kind === "window" ? (
              <div className="max-h-[168px] overflow-auto rounded-lg border border-line divide-y divide-[var(--border)] mb-3">
                {sources?.windows.length === 0 && <div className="p-3 text-muted">Brak okien do przechwycenia.</div>}
                {sources?.windows.map((w) => (
                  <button
                    key={w.id}
                    onClick={() => { setWindowId(w.id); setCrop(null); }}
                    className={cx("w-full flex items-center gap-3 px-3 py-2 text-left", w.id === windowId ? "bg-accent/10" : "hover:bg-panel-2")}
                  >
                    <AppWindow size={14} className={w.id === windowId ? "text-accent" : "text-subtle"} />
                    <span className="font-medium shrink-0">{w.app_name}</span>
                    <span className="text-muted truncate">{w.title || "(bez tytułu)"}</span>
                    {!w.on_screen && <Badge>poza ekranem</Badge>}
                    {/teams/i.test(w.app_name) && <Badge tone="accent">Teams</Badge>}
                  </button>
                ))}
              </div>
            ) : (
              <div className="mb-3">
                <Select
                  value={String(displayId ?? "")}
                  onChange={(v) => { setDisplayId(Number(v)); setCrop(null); }}
                  options={(sources?.displays ?? []).map((d) => ({ value: String(d.id), label: d.name }))}
                />
              </div>
            )}
            <div className="flex items-center justify-between mb-2">
              <div className="text-[12px] text-muted flex items-center gap-1.5">
                <Crop size={13} /> Przeciągnij na podglądzie, aby zaznaczyć sam obszar prezentacji (bez panelu uczestników i paska Teams).
              </div>
              {crop && <Button size="sm" variant="ghost" onClick={() => setCrop(null)}>Cały obraz</Button>}
            </div>
            {preview ? (
              <CropSelector src={preview.data_url} crop={crop} onChange={setCrop} nativeW={preview.width} nativeH={preview.height} />
            ) : (
              <div className="aspect-video rounded-lg border border-dashed border-line flex items-center justify-center text-muted">
                {previewLoading ? "Wczytywanie podglądu…" : "Brak podglądu – wybierz źródło"}
              </div>
            )}
            {preview && (
              <div className="text-[11.5px] text-subtle mt-2">
                Natywna rozdzielczość źródła: {preview.width} × {preview.height} px. Slajdy zapisywane są bezstratnie (PNG) bez skalowania.
              </div>
            )}
          </Card>
        </div>

        <div className="space-y-4">
          <Card title="Wykład">
            <div className="space-y-3">
              <Field label="Przedmiot (folder)">
                <Select
                  value={subject}
                  onChange={setSubject}
                  options={[
                    { value: "", label: "Bez przedmiotu" },
                    ...subjects.map((x) => ({ value: x.name, label: x.name })),
                    ...(subject && subject !== NEW_SUBJECT && !subjects.some((x) => x.name === subject) ? [{ value: subject, label: subject }] : []),
                    { value: NEW_SUBJECT, label: "+ Nowy przedmiot…" },
                  ]}
                />
              </Field>
              {subject === NEW_SUBJECT && (
                <Field label="Nazwa nowego przedmiotu">
                  <Input autoFocus placeholder="np. Algorytmy i struktury danych" value={newSubject} onChange={(e) => setNewSubject(e.target.value)} />
                </Field>
              )}
              <Field label="Temat wykładu">
                <Input autoFocus={subject !== NEW_SUBJECT} placeholder={subjectName ? `np. Wykład 3 – drzewa binarne` : "np. Algorytmy i struktury danych"} value={title} onChange={(e) => setTitle(e.target.value)} />
              </Field>
              <Field label="Lokalizacja zapisu" hint={<span className="break-all">Zapis do: {savePath} · wolne: {fmtBytes(free)}</span>}>
                <div className="flex gap-2">
                  <Input value={outputDir} onChange={(e) => setOutputDir(e.target.value)} className="flex-1 text-[12px]" />
                  <Button
                    icon={<FolderOpen size={14} />}
                    onClick={async () => {
                      const d = await openDialog({ directory: true, defaultPath: outputDir });
                      if (typeof d === "string") setOutputDir(d);
                    }}
                  />
                </div>
              </Field>
            </div>
          </Card>

          <Card title="Koniec wykładu">
            <AutoStopFields
              enabled={stopEnabled}
              setEnabled={setStopEnabled}
              time={stopAt}
              setTime={setStopAt}
              leave={leave}
              setLeave={setLeave}
              info={autoInfo}
            />
          </Card>

          <Card title="Dźwięk">
            <div className="space-y-3">
              <Toggle
                checked={audio.capture_system}
                onChange={(v) => setAudio({ ...audio, capture_system: v })}
                label={<span className="flex items-center gap-1.5"><Volume2 size={14} /> Dźwięk systemowy (Teams)</span>}
                description={platform === "macos" ? "ScreenCaptureKit – działa także na słuchawkach; dźwięk tej aplikacji jest wykluczony." : "WASAPI loopback wybranego wyjścia audio."}
              />
              {audio.capture_system && platform === "macos" && (
                <Field label="Źródło dźwięku systemowego">
                  <Select
                    value={audio.only_application ?? ""}
                    onChange={(v) => setAudio({ ...audio, only_application: v || null })}
                    options={[{ value: "", label: "Wszystkie aplikacje" }, ...apps.map(([b, n]) => ({ value: b, label: `Tylko ${n}` }))]}
                  />
                </Field>
              )}
              {audio.capture_system && platform === "windows" && (
                <Field label="Urządzenie wyjściowe (loopback)">
                  <Select
                    value={audio.loopback_device ?? ""}
                    onChange={(v) => setAudio({ ...audio, loopback_device: v || null })}
                    options={[{ value: "", label: "Domyślne wyjście" }, ...(devices?.outputs ?? []).map((d) => ({ value: d.name, label: d.name }))]}
                  />
                </Field>
              )}
              <Toggle
                checked={audio.capture_microphone}
                onChange={(v) => setAudio({ ...audio, capture_microphone: v })}
                label={<span className="flex items-center gap-1.5"><Mic size={14} /> Mój mikrofon</span>}
                description="Opcjonalnie – np. gdy zadajesz pytania. Używaj słuchawek, aby uniknąć echa."
              />
              {audio.capture_microphone && (
                <Select
                  value={audio.microphone_device ?? ""}
                  onChange={(v) => setAudio({ ...audio, microphone_device: v || null })}
                  options={[{ value: "", label: "Domyślny mikrofon" }, ...(devices?.inputs ?? []).map((d) => ({ value: d.name, label: d.name }))]}
                />
              )}
              <div className="pt-1">
                <Button size="sm" icon={<Play size={13} />} loading={testing} onClick={runTest} disabled={!audio.capture_system && !audio.capture_microphone}>
                  Test dźwięku (3 s)
                </Button>
                {test && (
                  <div className="mt-2.5 space-y-2">
                    {test.map((t, i) => (
                      <div key={i} className="text-[12px]">
                        <div className="flex justify-between mb-1">
                          <span className="font-medium">{t.description}</span>
                          <span className="text-muted tabular-nums">{t.started ? `${t.level_db.toFixed(0)} dB` : "błąd"}</span>
                        </div>
                        {t.started && <LevelMeter db={t.level_db} />}
                        <div className={cx("mt-1", t.error || t.buffers === 0 || t.level_db < -60 ? "text-amber-600 dark:text-amber-400" : "text-emerald-600 dark:text-emerald-400")}>
                          {t.error
                            ? t.error
                            : t.buffers === 0
                              ? "Nie otrzymano żadnych danych audio – sprawdź uprawnienia i urządzenie."
                              : t.level_db < -60
                                ? `Dane docierają (${t.seconds_received.toFixed(1)} s), ale to cisza. Włącz dźwięk w Teams i spróbuj ponownie.`
                                : `Sygnał OK – ${t.seconds_received.toFixed(1)} s audio odebrane.`}
                        </div>
                      </div>
                    ))}
                  </div>
                )}
              </div>
            </div>
          </Card>

          <Card title="Transkrypcja">
            <div className="space-y-3">
              <Toggle checked={transcribe} onChange={setTranscribe} label="Transkrybuj lokalnie (whisper.cpp)" />
              {transcribe && (
                <>
                  <div className="grid grid-cols-2 gap-2">
                    <Field label="Model">
                      <Select
                        value={model}
                        onChange={setModel}
                        options={models.map((m) => ({ value: m.info.id, label: `${m.info.label.replace(" (multilingual)", "")}${m.installed ? "" : " – nie pobrano"}` }))}
                      />
                    </Field>
                    <Field label="Język">
                      <Select value={language} onChange={setLanguage} options={LANGUAGES.map((l) => ({ value: l.id, label: l.label }))} />
                    </Field>
                  </div>
                  {modelMissing && (
                    <div className="flex items-center justify-between gap-2 rounded-lg bg-amber-500/5 border border-amber-500/30 p-2.5 text-[12px]">
                      <span>Model nie jest pobrany ({fmtBytes(selectedModel!.info.size)}).</span>
                      {downloads[model]?.state === "running" ? (
                        <span className="tabular-nums text-muted">{Math.round((downloads[model].downloaded / Math.max(1, downloads[model].total)) * 100)}%</span>
                      ) : (
                        <Button size="sm" variant="primary" icon={<Download size={13} />} onClick={() => api.downloadModel(model)}>Pobierz</Button>
                      )}
                    </div>
                  )}
                  <Toggle
                    checked={live}
                    onChange={setLive}
                    label="Na bieżąco podczas wykładu"
                    description="Wyłącz, aby transkrybować dopiero po zakończeniu (mniejsze obciążenie). Audio i slajdy zawsze mają pierwszeństwo."
                  />
                </>
              )}
            </div>
          </Card>
          <div className="flex items-start gap-2 text-[11.5px] text-subtle px-1">
            <Info size={13} className="shrink-0 mt-0.5" />
            Pamiętaj o zasadach uczelni i zgodzie prowadzącego na nagrywanie.
          </div>
        </div>
      </div>

      <div className="fixed bottom-0 left-[214px] right-0 border-t border-line bg-panel/90 backdrop-blur px-8 py-3 flex items-center justify-end gap-3">
        <span className="text-muted text-[12.5px] mr-auto">
          {target ? (crop ? "Nagrywany będzie zaznaczony obszar." : "Nagrywany będzie cały wybrany obraz.") : "Wybierz źródło obrazu."}
        </span>
        <Button variant="ghost" onClick={() => nav.go({ name: "dashboard" })}>Anuluj</Button>
        <Button variant="rec" size="lg" loading={starting} disabled={nothingToRecord || modelMissing} onClick={start}>
          <span className="w-2.5 h-2.5 rounded-full bg-white" /> Rozpocznij nagrywanie
        </Button>
      </div>
    </div>
  );
}

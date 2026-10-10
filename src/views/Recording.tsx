import { useEffect, useState } from "react";
import { Pause, Play, Camera, Square, Image as ImageIcon, Mic, MicOff, AlertTriangle, Info, XCircle, Clock } from "lucide-react";
import { api, AutoStopInfo, fileSrc, RecorderStatus } from "../lib/api";
import { AutoStopFields } from "./NewLecture";
import { errorText, fmtBytes, fmtDuration, fmtMs, plural } from "../lib/format";
import { Badge, Button, Card, LevelMeter, Modal, useToast, cx } from "../components/ui";
import type { Nav } from "../App";

function Stat({ label, value, sub }: { label: string; value: React.ReactNode; sub?: React.ReactNode }) {
  return (
    <div>
      <div className="text-[11.5px] text-muted font-medium">{label}</div>
      <div className="text-[18px] font-semibold tabular-nums mt-0.5">{value}</div>
      {sub && <div className="text-[11.5px] text-subtle mt-0.5">{sub}</div>}
    </div>
  );
}

function hhmm(iso: string) {
  const d = new Date(iso);
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

function AutoStopCard({ nav, audioOnly }: { nav: Nav; audioOnly: boolean }) {
  const toast = useToast();
  const [info, setInfo] = useState<AutoStopInfo | null>(null);
  const [editing, setEditing] = useState(false);
  const [enabled, setEnabled] = useState(false);
  const [time, setTime] = useState("21:00");
  const [leave, setLeave] = useState(nav.settings.auto_leave_meeting);
  const [now, setNow] = useState(Date.now());

  useEffect(() => {
    api.getAutoStop().then((i) => {
      setInfo(i);
      if (i.schedule) {
        setEnabled(true);
        setTime(hhmm(i.schedule.at));
        setLeave(i.schedule.leave_meeting);
      }
    });
    const t = setInterval(() => setNow(Date.now()), 15_000);
    return () => clearInterval(t);
  }, []);

  const save = async () => {
    try {
      const i = await api.setAutoStop(enabled ? time : null, leave && !audioOnly);
      setInfo(i);
      setEditing(false);
      toast(i.schedule ? `Nagranie zakończy się o ${hhmm(i.schedule.at)}.` : "Wyłączono automatyczne zakończenie.", "success");
    } catch (e) {
      toast(errorText(e), "error");
    }
  };

  const sch = info?.schedule;
  const left = sch ? Math.max(0, new Date(sch.at).getTime() - now) : 0;
  return (
    <Card title="Koniec wykładu" actions={!editing && <Button size="sm" variant="ghost" onClick={() => setEditing(true)}>{sch ? "Zmień" : "Ustaw"}</Button>}>
      {editing ? (
        <div className="space-y-3">
          <AutoStopFields enabled={enabled} setEnabled={setEnabled} time={time} setTime={setTime} leave={leave} setLeave={setLeave} info={info} hideLeave={audioOnly} />
          <div className="flex justify-end gap-2">
            <Button size="sm" variant="ghost" onClick={() => setEditing(false)}>Anuluj</Button>
            <Button size="sm" variant="primary" onClick={save}>Zapisz</Button>
          </div>
        </div>
      ) : sch ? (
        <div className="text-[12.5px] space-y-1">
          <div className="flex items-center gap-2 font-medium"><Clock size={14} className="text-accent" /> o {hhmm(sch.at)} <span className="text-muted font-normal">(za {fmtDuration(left)})</span></div>
          <div className="text-muted">{sch.leave_meeting ? `Zapisze wykład i opuści spotkanie Teams (${info?.shortcut}).` : audioOnly ? "Zapisze wykład." : "Zapisze wykład (bez opuszczania spotkania)."}</div>
        </div>
      ) : (
        <div className="text-muted text-[12.5px]">Nagrywasz do ręcznego zatrzymania.</div>
      )}
    </Card>
  );
}

const whisperLabel: Record<string, string> = {
  disabled: "wyłączona",
  waiting: "po wykładzie",
  loading: "ładowanie modelu…",
  running: "rozpoznaje",
  idle: "czeka na mowę",
  done: "zakończona",
  failed: "błąd",
  cancelled: "anulowana",
};

export function Recording({ nav }: { nav: Nav }) {
  const toast = useToast();
  const [st, setSt] = useState<RecorderStatus | null>(null);
  const [confirmStop, setConfirmStop] = useState(false);
  const [stopping, setStopping] = useState(false);
  const [micOn, setMicOn] = useState(true);

  useEffect(() => {
    let alive = true;
    const tick = async () => {
      const s = await api.recordingStatus();
      if (alive) setSt(s);
    };
    tick();
    const t = setInterval(tick, 500);
    return () => {
      alive = false;
      clearInterval(t);
    };
  }, []);

  useEffect(() => {
    const h = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key.toLowerCase() === "s") e.preventDefault();
    };
    window.addEventListener("keydown", h);
    return () => window.removeEventListener("keydown", h);
  }, []);

  if (!st) {
    return (
      <div className="max-w-[900px] mx-auto px-8">
        <Card>
          <div className="py-10 text-center text-muted">Nagrywanie nie jest aktywne.</div>
          <div className="flex justify-center"><Button variant="primary" onClick={() => nav.go({ name: "new" })}>Nowy wykład</Button></div>
        </Card>
      </div>
    );
  }

  const paused = st.state === "paused";
  const hasMic = st.audio.sources.some((s) => s.kind === "microphone");
  const sys = st.audio.sources.find((s) => s.kind === "system");
  const mic = st.audio.sources.find((s) => s.kind === "microphone");
  const audioOnly = st.audio_only;
  // the source the lecture is heard through: system audio (Teams) or the microphone (hall)
  const main = audioOnly ? mic : sys;
  const t = st.transcription;

  const stop = async () => {
    setStopping(true);
    try {
      const path = await api.stopRecording();
      toast("Wykład zapisany. Transkrypcja jest dokańczana w tle.", "success");
      nav.go({ name: "lecture", path });
    } catch (e) {
      toast(errorText(e), "error");
    } finally {
      setStopping(false);
      setConfirmStop(false);
    }
  };

  return (
    <div className="max-w-[1180px] mx-auto px-8 pb-10">
      <div className="flex items-center gap-5 mb-5">
        <div className={cx("flex items-center gap-2.5 px-3 h-8 rounded-full border font-semibold text-[12px] tracking-wide",
          paused ? "border-amber-500/40 text-amber-600 dark:text-amber-400 bg-amber-500/10" : "border-rec/40 text-rec bg-rec/10")}>
          <span className={cx("w-2.5 h-2.5 rounded-full", paused ? "bg-amber-500" : "bg-rec rec-dot")} />
          {paused ? "PAUZA" : "NAGRYWANIE"}
        </div>
        <div className="text-[34px] font-semibold tabular-nums tracking-tight">{fmtMs(st.elapsed_ms)}</div>
        <div className="min-w-0">
          <div className="font-semibold truncate">{st.title}</div>
          <div className="text-[11.5px] text-subtle truncate selectable">{st.lecture_dir}</div>
        </div>
        <div className="ml-auto flex gap-2">
          {paused ? (
            <Button size="lg" icon={<Play size={15} />} onClick={() => api.resume()}>Wznów</Button>
          ) : (
            <Button size="lg" icon={<Pause size={15} />} onClick={() => api.pause()}>Pauza</Button>
          )}
          {!audioOnly && (
            <Button size="lg" icon={<Camera size={15} />} onClick={() => api.captureSlide()} title="Skrót: ⌘⇧S / Ctrl+Shift+S" disabled={paused}>
              Zapisz slajd
            </Button>
          )}
          <Button size="lg" variant="rec" icon={<Square size={14} fill="currentColor" />} onClick={() => setConfirmStop(true)}>
            Zatrzymaj i zapisz
          </Button>
        </div>
      </div>

      <div className="grid grid-cols-[minmax(0,1fr)_360px] gap-4">
        <div className="space-y-4 min-w-0">
          {!audioOnly && <Card title={`Ostatni slajd${st.last_slide_id ? ` · #${st.last_slide_id}` : ""}`}>
            <div className="aspect-video rounded-lg bg-panel-2 border border-line overflow-hidden flex items-center justify-center">
              {st.last_slide_path ? (
                <img src={fileSrc(st.last_slide_path, `${st.slides}-${st.occurrences}-${st.last_slide_id}`)} className="w-full h-full object-contain" alt="" />
              ) : (
                <div className="text-subtle flex flex-col items-center gap-2"><ImageIcon size={26} /> Czekam na pierwszy stabilny slajd…</div>
              )}
            </div>
          </Card>}
          <Card title="Ostatnie wypowiedzi">
            {st.recent_segments.length === 0 ? (
              <div className="text-muted text-[12.5px] py-2">
                {t.state === "waiting" ? "Transkrypcja rozpocznie się po zakończeniu wykładu." : t.state === "disabled" ? "Transkrypcja jest wyłączona." : "Pierwsze zdania pojawią się po chwili mowy."}
              </div>
            ) : (
              <div className={cx("space-y-2 overflow-auto selectable", audioOnly ? "max-h-[560px]" : "max-h-[260px]")}>
                {st.recent_segments.slice().reverse().map((s, i) => (
                  <div key={i} className={cx(audioOnly ? "text-[14px] leading-relaxed" : "text-[13px] leading-relaxed", i > 0 && "text-muted")}>
                    <span className="text-subtle tabular-nums mr-2 text-[11.5px]">{fmtMs(s.start_ms)}</span>{s.text}
                  </div>
                ))}
              </div>
            )}
          </Card>
        </div>

        <div className="space-y-4">
          <Card title="Status">
            <div className="grid grid-cols-2 gap-4 mb-4">
              {audioOnly
                ? <Stat label="Nagrany dźwięk" value={fmtMs(st.audio.recorded_ms)} sub={`mowa ${Math.round(st.audio.speech_ratio * 100)}%`} />
                : <Stat label="Slajdy" value={st.slides} sub={plural(st.occurrences, "wyświetlenie", "wyświetlenia", "wyświetleń")} />}
              <Stat label="Na dysku" value={fmtBytes(st.lecture_bytes)} sub={`wolne: ${fmtBytes(st.free_bytes)}`} />
            </div>
            <div className="space-y-3 text-[12.5px]">
              <div className="flex items-center justify-between">
                <span className="text-muted">Obraz</span>
                {audioOnly ? <Badge>wyłączony · tryb „Na sali”</Badge>
                  : st.video.state === "ok" ? <Badge tone="green">przechwytywanie · {st.video.width}×{st.video.height}</Badge>
                  : st.video.state === "lost" ? <Badge tone="red">utracony</Badge> : <Badge>uruchamianie</Badge>}
              </div>
              {st.video.detail && <div className="text-red-500 text-[12px]">{st.video.detail}</div>}
              <div>
                <div className="flex items-center justify-between mb-1.5">
                  <span className="text-muted">Dźwięk</span>
                  <span className="tabular-nums text-muted">{st.audio.level_db.toFixed(0)} dB</span>
                </div>
                <LevelMeter db={st.audio.level_db} />
                <div className="flex items-center justify-between mt-1.5 text-[11.5px]">
                  <span className={st.audio.no_signal || !main || main.received_samples === 0 ? "text-amber-600 dark:text-amber-400" : "text-subtle"}>
                    {!main ? (audioOnly ? "brak mikrofonu" : "brak źródła systemowego")
                      : main.received_samples === 0 ? (audioOnly ? "brak danych z mikrofonu" : "brak danych audio z systemu")
                        : st.audio.no_signal ? `cisza od ${Math.round(st.audio.silent_for_ms / 1000)} s`
                          : `dane docierają · mowa ${Math.round(st.audio.speech_ratio * 100)}%`}
                  </span>
                  {hasMic && !audioOnly && (
                    <button
                      className="flex items-center gap-1 text-muted hover:text-fg"
                      onClick={() => { api.setMicrophoneEnabled(!micOn); setMicOn(!micOn); }}
                    >
                      {micOn ? <Mic size={12} /> : <MicOff size={12} />} mikrofon {micOn ? "wł." : "wył."}
                    </button>
                  )}
                </div>
              </div>
              <div className="flex items-center justify-between">
                <span className="text-muted">Whisper</span>
                <Badge tone={t.state === "failed" ? "red" : t.state === "running" ? "blue" : "neutral"}>
                  {whisperLabel[t.state] ?? t.state}{t.model ? ` · ${t.model}` : ""}
                </Badge>
              </div>
              {t.state !== "disabled" && (
                <div className="flex items-center justify-between text-[12px] text-muted">
                  <span>Kolejka: {t.queue_len} · gotowe: {t.done}{t.failed ? ` · błędy: ${t.failed}` : ""}</span>
                  <span className="tabular-nums">{t.speed > 0 ? `${t.speed.toFixed(1)}× RT` : ""}{t.lag_ms > 60000 ? ` · opóźn. ${fmtMs(t.lag_ms)}` : ""}</span>
                </div>
              )}
              {t.error && <div className="text-red-500 text-[12px] selectable">{t.error}</div>}
            </div>
          </Card>
          <AutoStopCard nav={nav} audioOnly={audioOnly} />
          <Card title="Komunikaty">
            {st.warnings.length === 0 ? (
              <div className="text-muted text-[12.5px]">Wszystko działa poprawnie.</div>
            ) : (
              <div className="space-y-2 max-h-[240px] overflow-auto selectable">
                {st.warnings.slice().reverse().map((w, i) => (
                  <div key={i} className="flex gap-2 text-[12.5px]">
                    {w.level === "error" ? <XCircle size={14} className="text-red-500 shrink-0 mt-0.5" />
                      : w.level === "warning" ? <AlertTriangle size={14} className="text-amber-500 shrink-0 mt-0.5" />
                        : <Info size={14} className="text-sky-500 shrink-0 mt-0.5" />}
                    <div><span className="text-subtle tabular-nums mr-1.5">{fmtMs(w.t_ms)}</span>{w.message}</div>
                  </div>
                ))}
              </div>
            )}
          </Card>
        </div>
      </div>

      <Modal
        open={confirmStop}
        onClose={() => setConfirmStop(false)}
        title="Zakończyć nagrywanie?"
        footer={<>
          <Button variant="ghost" onClick={() => setConfirmStop(false)}>Kontynuuj nagrywanie</Button>
          <Button variant="rec" loading={stopping} onClick={stop}>Zatrzymaj i zapisz</Button>
        </>}
      >
        <p className="text-muted leading-relaxed">
          {audioOnly ? "Dźwięk zostanie zapisany" : "Slajdy i dźwięk zostaną zapisane"}, a dokumenty (lecture.md, transkrypcja, PROMPT.md) wygenerowane. Pozostała transkrypcja
          zostanie dokończona w tle – możesz od razu przeglądać materiały.
        </p>
      </Modal>
    </div>
  );
}

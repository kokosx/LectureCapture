import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { openPath, revealItemInDir } from "@tauri-apps/plugin-opener";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import {
  ArrowLeft, FolderOpen, ClipboardCopy, FileArchive, RotateCw, ChevronLeft, ChevronRight, Trash2, Clock, Layers,
  AlertTriangle, Pencil, Check, X, FileText, Play, Folder, School,
} from "lucide-react";
import { api, fileSrc, LANGUAGES, LectureDetail, TranscriptionInfo, Part, SubjectInfo } from "../lib/api";
import { errorText, fmtBytes, fmtDateTime, fmtDuration, fmtMs, parseTime, plural, transcriptionLabel } from "../lib/format";
import { Badge, Button, Card, Empty, Input, Modal, Progress, Segmented, Select, Toggle, useToast, cx } from "../components/ui";
import { transcriptionTone } from "./Dashboard";
import { useModels } from "./Models";
import type { Nav } from "../App";

const gapLabel: Record<string, string> = {
  paused: "pauza",
  video_lost: "utracony obraz",
  audio_lost: "utracone audio",
  no_audio_signal: "brak sygnału audio",
  crash: "przerwanie nagrywania",
  transcription_failed: "nieudana transkrypcja",
};

function PartsView({ parts, empty }: { parts: Part[]; empty?: string }) {
  if (parts.length === 0) return <div className="text-muted italic text-[12.5px]">{empty ?? "Brak wypowiedzi."}</div>;
  return (
    <div className="space-y-2.5 selectable">
      {parts.map((p, i) => (
        <p key={i} className="leading-relaxed text-[13.5px]">
          <span className="text-subtle tabular-nums text-[11.5px] mr-2">{fmtMs(p.start_ms)}</span>
          {p.continues_from_prev && <span className="text-subtle">… </span>}
          {p.words.map((w) => w.w).join(" ").replace(/ ([,.;:!?)])/g, "$1")}
          {p.continues_to_next && <span className="text-subtle"> …</span>}
        </p>
      ))}
    </div>
  );
}

export function LectureDetailView({ nav, path }: { nav: Nav; path: string }) {
  const toast = useToast();
  const [d, setD] = useState<LectureDetail | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [tab, setTab] = useState<"timeline" | "gallery" | "transcript" | "info">("timeline");
  const [sel, setSel] = useState(0);
  const [ts, setTs] = useState<TranscriptionInfo | null>(null);
  const [editTitle, setEditTitle] = useState<string | null>(null);
  const [boundary, setBoundary] = useState("");
  const [confirmDelete, setConfirmDelete] = useState<number | null>(null);
  const [retx, setRetx] = useState(false);
  const [retxModel, setRetxModel] = useState(nav.settings.transcription.model);
  const [retxLang, setRetxLang] = useState(nav.settings.transcription.language);
  const [zipAudio, setZipAudio] = useState(false);
  const [zipOpen, setZipOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const { models } = useModels();
  const wasRunning = useRef(false);
  const [subjects, setSubjects] = useState<SubjectInfo[]>([]);

  useEffect(() => {
    api.listSubjects().then(setSubjects).catch(() => {});
  }, []);

  // recorded in the lecture hall: no slides, the transcript is the main view
  const audioOnly = d?.manifest.capture.source.kind === "audio";
  useEffect(() => {
    if (audioOnly && (tab === "timeline" || tab === "gallery")) setTab("transcript");
  }, [audioOnly, tab]);

  const load = useCallback(async () => {
    try {
      const det = await api.getLecture(path);
      setD(det);
      setError(null);
    } catch (e) {
      setError(errorText(e));
    }
  }, [path]);

  useEffect(() => {
    load();
  }, [load]);

  // poll background transcription for this lecture
  useEffect(() => {
    const t = setInterval(async () => {
      const s = await api.transcriptionStatus(path);
      setTs(s);
      const running = (s.service && !["done", "failed", "cancelled"].includes(s.service.state)) || s.job?.state === "running";
      if (wasRunning.current && !running) load();
      wasRunning.current = !!running;
    }, 1000);
    return () => clearInterval(t);
  }, [path, load]);

  const tl = d?.assignment.slides ?? [];
  const cur = tl[Math.min(sel, Math.max(0, tl.length - 1))];
  useEffect(() => {
    if (cur) setBoundary(fmtMs(cur.start_ms));
  }, [cur?.occurrence_id, cur?.start_ms]);

  useEffect(() => {
    const h = (e: KeyboardEvent) => {
      if (tab !== "timeline" || (e.target as HTMLElement).tagName === "INPUT") return;
      if (e.key === "ArrowDown" || e.key === "ArrowRight") setSel((s) => Math.min(s + 1, tl.length - 1));
      if (e.key === "ArrowUp" || e.key === "ArrowLeft") setSel((s) => Math.max(s - 1, 0));
    };
    window.addEventListener("keydown", h);
    return () => window.removeEventListener("keydown", h);
  }, [tab, tl.length]);

  const occurrencesBySlide = useMemo(() => {
    const m = new Map<number, number>();
    tl.forEach((o) => m.set(o.slide_id, (m.get(o.slide_id) ?? 0) + 1));
    return m;
  }, [tl]);

  if (error) {
    return <div className="max-w-[900px] mx-auto px-8"><Card><Empty icon={<AlertTriangle size={26} />} title="Nie można otworzyć wykładu">{error}</Empty></Card></div>;
  }
  if (!d) return <div className="px-8 text-muted">Wczytywanie…</div>;
  const m = d.manifest;
  const version = m.slides.map((s) => s.sha256.slice(0, 6)).join("");
  const running = (ts?.service && !["done", "failed", "cancelled"].includes(ts.service.state)) || ts?.job?.state === "running";

  const act = async (fn: () => Promise<unknown>, ok?: string) => {
    setBusy(true);
    try {
      await fn();
      if (ok) toast(ok, "success");
      await load();
    } catch (e) {
      toast(errorText(e), "error");
    } finally {
      setBusy(false);
    }
  };

  const slidePath = (id: number) => d.slide_paths[String(id)];

  return (
    <div className="max-w-[1240px] mx-auto px-8 pb-10">
      <div className="flex items-center gap-3 mb-3">
        <button onClick={() => nav.go({ name: "dashboard" })} className="flex items-center gap-1 text-muted hover:text-fg text-[12.5px]">
          <ArrowLeft size={14} /> Wykłady
        </button>
        <div className="ml-auto flex items-center gap-2 text-[12px] text-muted">
          <Folder size={13} />
          <span>Przedmiot:</span>
          <Select
            className="h-7 w-60 text-[12.5px]"
            disabled={busy || running}
            value={d.subject ?? ""}
            onChange={async (v) => {
              setBusy(true);
              try {
                const np = await api.moveLecture(path, v || null);
                toast(v ? `Przeniesiono do „${v}”.` : "Wykład nie jest już przypisany do przedmiotu.", "success");
                nav.go({ name: "lecture", path: np });
              } catch (e) {
                toast(errorText(e), "error");
              } finally {
                setBusy(false);
              }
            }}
            options={[
              { value: "", label: "Bez przedmiotu" },
              ...subjects.map((x) => ({ value: x.name, label: x.name })),
              ...(d.subject && !subjects.some((x) => x.name === d.subject) ? [{ value: d.subject, label: d.subject }] : []),
            ]}
          />
        </div>
      </div>
      <div className="flex items-start gap-4 mb-4">
        <div className="min-w-0 flex-1">
          {editTitle !== null ? (
            <div className="flex items-center gap-2">
              <Input autoFocus value={editTitle} onChange={(e) => setEditTitle(e.target.value)} className="text-[18px] h-10 font-semibold max-w-xl" />
              <Button icon={<Check size={14} />} onClick={() => act(() => api.renameLecture(path, editTitle).then(() => setEditTitle(null)))} />
              <Button variant="ghost" icon={<X size={14} />} onClick={() => setEditTitle(null)} />
            </div>
          ) : (
            <h1 className="text-[22px] font-semibold tracking-tight flex items-center gap-2 group">
              <span className="truncate">{m.lecture.title}</span>
              <button className="opacity-0 group-hover:opacity-100 text-muted" onClick={() => setEditTitle(m.lecture.title)}><Pencil size={14} /></button>
            </h1>
          )}
          <div className="flex flex-wrap items-center gap-2 mt-1.5 text-[12.5px] text-muted">
            <span>{fmtDateTime(m.lecture.started_at)}</span>
            <Badge><Clock size={11} /> {fmtDuration(m.lecture.duration_ms ?? m.lecture.last_alive_ms)}</Badge>
            {audioOnly
              ? <Badge><School size={11} /> na sali · tylko dźwięk</Badge>
              : <Badge><Layers size={11} /> {plural(m.slides.length, "slajd", "slajdy", "slajdów")} · {plural(m.timeline.length, "wyświetlenie", "wyświetlenia", "wyświetleń")}</Badge>}
            <Badge tone={transcriptionTone(m.transcription.status)}>transkrypcja: {transcriptionLabel[m.transcription.status]}</Badge>
            {m.lecture.status === "recovered" && <Badge tone="yellow">odzyskany po awarii</Badge>}
            <span>{fmtBytes(d.size_bytes)}</span>
          </div>
        </div>
        <div className="flex gap-2 shrink-0 flex-wrap justify-end">
          <Button icon={<FolderOpen size={14} />} onClick={() => revealItemInDir(path + "/manifest.json").catch(() => openPath(path))}>Pokaż folder</Button>
          <Button
            variant="primary"
            icon={<ClipboardCopy size={14} />}
            onClick={async () => {
              try {
                const p = await api.readPrompt(path);
                await writeText(p);
                toast("PROMPT.md skopiowany. Wklej go agentowi (Claude Code / Cowork / ChatGPT) z dostępem do tego folderu.", "success");
              } catch (e) {
                toast(errorText(e), "error");
              }
            }}
          >
            Kopiuj prompt
          </Button>
          <Button icon={<FileArchive size={14} />} onClick={() => setZipOpen(true)}>Eksport ZIP</Button>
          <Button icon={<RotateCw size={14} />} disabled={running || !d.has_audio} title={d.has_audio ? "" : "Nagranie audio zostało usunięte"} onClick={() => setRetx(true)}>
            Transkrybuj ponownie
          </Button>
        </div>
      </div>

      {running && (
        <Card className="mb-4">
          <div className="flex items-center gap-4">
            <div className="text-[12.5px] font-medium w-56">
              {ts?.job ? `Ponowna transkrypcja… ${Math.round((ts.job.progress ?? 0) * 100)}%` : `Transkrypcja w tle · ${ts?.service?.done ?? 0} gotowe, ${ts?.service?.queue_len ?? 0} w kolejce`}
            </div>
            <Progress
              className="flex-1"
              value={ts?.job ? ts.job.progress : (ts?.service?.done ?? 0) / Math.max(1, (ts?.service?.done ?? 0) + (ts?.service?.queue_len ?? 0))}
            />
            <Button size="sm" variant="ghost" onClick={() => api.cancelTranscription(path)}>Anuluj</Button>
          </div>
          {(ts?.job?.last_text || ts?.service?.last_text) && (
            <div className="text-muted text-[12px] mt-2 truncate">„{ts?.job?.last_text ?? ts?.service?.last_text}”</div>
          )}
        </Card>
      )}
      {!running && (d.pending_chunks > 0 || d.failed_chunks > 0) && (
        <Card className="mb-4">
          <div className="flex items-center gap-3 text-[12.5px]">
            <AlertTriangle size={16} className="text-amber-500" />
            <span className="flex-1">
              {d.pending_chunks > 0 && `${plural(d.pending_chunks, "fragment czeka", "fragmenty czekają", "fragmentów czeka")} na transkrypcję. `}
              {d.failed_chunks > 0 && `${d.failed_chunks} fragmentów nie udało się rozpoznać. `}
              {m.transcription.error && <span className="text-red-500">{m.transcription.error}</span>}
            </span>
            <Button size="sm" variant="primary" icon={<Play size={13} />} onClick={() => act(() => api.resumeTranscription(path), "Wznowiono transkrypcję")}>
              Dokończ transkrypcję
            </Button>
          </div>
        </Card>
      )}

      <div className="mb-4">
        <Segmented
          value={tab}
          onChange={setTab}
          options={[
            ...(audioOnly ? [] : [
              { value: "timeline" as const, label: "Slajdy i wypowiedzi" },
              { value: "gallery" as const, label: `Galeria (${m.slides.length})` },
            ]),
            { value: "transcript", label: "Pełna transkrypcja" },
            { value: "info", label: "Informacje" },
          ]}
        />
      </div>

      {tab === "timeline" && (
        tl.length === 0 ? (
          <Card><Empty icon={<Layers size={26} />} title="Brak slajdów">Nie wykryto żadnego slajdu w tym nagraniu.</Empty></Card>
        ) : (
          <div className="grid grid-cols-[240px_minmax(0,1fr)] gap-4 items-start">
            <div className="bg-panel border border-line rounded-xl overflow-auto max-h-[calc(100vh-260px)] sticky top-0">
              {d.assignment.before_first.length > 0 && (
                <div className="px-3 py-2 text-[11.5px] text-subtle border-b border-line">Przed pierwszym slajdem: {d.assignment.before_first.length} wypowiedzi</div>
              )}
              {tl.map((o, i) => (
                <button
                  key={o.occurrence_id}
                  onClick={() => setSel(i)}
                  className={cx("w-full flex gap-2.5 p-2 text-left border-b border-line last:border-b-0", i === sel ? "bg-accent/10" : "hover:bg-panel-2")}
                >
                  <img src={fileSrc(slidePath(o.slide_id), version)} className="w-20 aspect-video object-cover rounded border border-line bg-panel-2" alt="" />
                  <div className="min-w-0">
                    <div className="font-medium text-[12.5px]">Slajd {o.slide_id}{(occurrencesBySlide.get(o.slide_id) ?? 0) > 1 && <span className="text-subtle font-normal"> ↺</span>}</div>
                    <div className="text-[11.5px] text-muted tabular-nums">{fmtMs(o.start_ms)}–{fmtMs(o.end_ms)}</div>
                    <div className="text-[11px] text-subtle">{plural(o.parts.reduce((a, p) => a + p.words.length, 0), "słowo", "słowa", "słów")}</div>
                  </div>
                </button>
              ))}
            </div>
            {cur && (
              <div className="space-y-4 min-w-0">
                <Card>
                  <div className="flex items-center justify-between mb-3">
                    <div className="flex items-center gap-2">
                      <Button size="sm" variant="ghost" icon={<ChevronLeft size={15} />} disabled={sel === 0} onClick={() => setSel(sel - 1)} />
                      <span className="font-semibold">Slajd {cur.slide_id}</span>
                      <span className="text-muted tabular-nums text-[12.5px]">{fmtMs(cur.start_ms)} – {fmtMs(cur.end_ms)}</span>
                      <Button size="sm" variant="ghost" icon={<ChevronRight size={15} />} disabled={sel >= tl.length - 1} onClick={() => setSel(sel + 1)} />
                    </div>
                    <div className="flex items-center gap-2">
                      <Button size="sm" variant="ghost" icon={<FileText size={13} />} onClick={() => openPath(slidePath(cur.slide_id))}>Otwórz</Button>
                      <Button size="sm" variant="danger" icon={<Trash2 size={13} />} disabled={running} onClick={() => setConfirmDelete(cur.slide_id)}>Usuń slajd</Button>
                    </div>
                  </div>
                  <img src={fileSrc(slidePath(cur.slide_id), version)} className="w-full rounded-lg border border-line bg-black" alt={`Slajd ${cur.slide_id}`} />
                  {sel > 0 && (
                    <div className="flex items-center gap-2 mt-3 text-[12.5px]">
                      <span className="text-muted">Początek wyświetlania (granica z poprzednim slajdem):</span>
                      {[-5000, -1000].map((dlt) => (
                        <Button key={dlt} size="sm" variant="ghost" onClick={() => setBoundary(fmtMs(Math.max(0, (parseTime(boundary) ?? cur.start_ms) + dlt)))}>{dlt / 1000}s</Button>
                      ))}
                      <Input value={boundary} onChange={(e) => setBoundary(e.target.value)} className="w-24 tabular-nums text-center" />
                      {[1000, 5000].map((dlt) => (
                        <Button key={dlt} size="sm" variant="ghost" onClick={() => setBoundary(fmtMs((parseTime(boundary) ?? cur.start_ms) + dlt))}>+{dlt / 1000}s</Button>
                      ))}
                      <Button
                        size="sm"
                        disabled={running || parseTime(boundary) === null || parseTime(boundary) === Math.floor(cur.start_ms / 1000) * 1000}
                        onClick={() => act(() => api.setOccurrenceStart(path, cur.occurrence_id, parseTime(boundary)!), "Granica przesunięta – dokumenty zaktualizowane")}
                      >
                        Zastosuj
                      </Button>
                    </div>
                  )}
                </Card>
                <Card title="Wypowiedzi prowadzącego przy tym slajdzie">
                  <PartsView parts={cur.parts} empty="Brak wypowiedzi przypisanych do tego slajdu." />
                </Card>
              </div>
            )}
          </div>
        )
      )}

      {tab === "gallery" && (
        <div className="grid grid-cols-[repeat(auto-fill,minmax(220px,1fr))] gap-3">
          {m.slides.map((s) => (
            <div key={s.id} className="bg-panel border border-line rounded-xl overflow-hidden group">
              <button className="block w-full" onClick={() => { const i = tl.findIndex((o) => o.slide_id === s.id); if (i >= 0) { setSel(i); setTab("timeline"); } }}>
                <img src={fileSrc(slidePath(s.id), version)} className="w-full aspect-video object-contain bg-black" alt="" />
              </button>
              <div className="p-2.5 flex items-center justify-between">
                <div>
                  <div className="font-medium text-[12.5px]">Slajd {s.id} {s.trigger === "manual" && <Badge>ręczny</Badge>} {s.updates > 0 && <Badge tone="blue">+{s.updates} etapy</Badge>}</div>
                  <div className="text-[11.5px] text-muted">{s.width}×{s.height} · {fmtBytes(s.bytes)} · {s.occurrences.length}× na osi</div>
                </div>
                <button className="opacity-0 group-hover:opacity-100 text-muted hover:text-red-500 p-1" disabled={running} onClick={() => setConfirmDelete(s.id)}><Trash2 size={14} /></button>
              </div>
            </div>
          ))}
        </div>
      )}

      {tab === "transcript" && (
        <Card>
          {d.records.length === 0 ? (
            <Empty icon={<FileText size={26} />} title="Brak transkrypcji">{m.transcription.status === "disabled" ? "Transkrypcja była wyłączona – możesz ją uruchomić przyciskiem „Transkrybuj ponownie”." : "Transkrypcja jeszcze nie jest gotowa."}</Empty>
          ) : (
            <div className="space-y-2 selectable max-w-[860px]">
              {d.records.map((r, i) => {
                const occ = tl.findIndex((o) => r.start_ms >= o.start_ms && r.start_ms < o.end_ms);
                return (
                  <p key={i} className="leading-relaxed text-[13.5px]">
                    <button
                      className="text-subtle hover:text-accent tabular-nums text-[11.5px] mr-2"
                      title={occ >= 0 ? `Przejdź do slajdu ${tl[occ].slide_id}` : ""}
                      onClick={() => { if (occ >= 0) { setSel(occ); setTab("timeline"); } }}
                    >
                      {fmtMs(r.start_ms)}{occ >= 0 ? ` · s${tl[occ].slide_id}` : ""}
                    </button>
                    {r.text}
                  </p>
                );
              })}
            </div>
          )}
        </Card>
      )}

      {tab === "info" && (
        <div className="grid grid-cols-2 gap-4">
          <Card title="Braki i jakość danych">
            {m.gaps.length === 0 && m.lecture.notes.length === 0 ? <div className="text-muted">Nie zarejestrowano braków danych.</div> : (
              <ul className="space-y-1.5 text-[12.5px] selectable">
                {m.gaps.map((g, i) => (
                  <li key={i}><span className="tabular-nums text-muted">{fmtMs(g.start_ms)}–{fmtMs(g.end_ms ?? m.lecture.duration_ms ?? 0)}</span> {gapLabel[g.kind] ?? g.kind}{g.detail ? ` — ${g.detail}` : ""}</li>
                ))}
                {m.lecture.notes.map((n, i) => <li key={`n${i}`}>{n}</li>)}
              </ul>
            )}
          </Card>
          <Card title="Szczegóły">
            <dl className="grid grid-cols-[140px_1fr] gap-y-1.5 text-[12.5px] selectable">
              {audioOnly ? (
                <><dt className="text-muted">Źródło</dt><dd>nagranie na sali – tylko dźwięk z mikrofonu</dd></>
              ) : (
                <>
                  <dt className="text-muted">Źródło</dt><dd>{m.capture.source.kind} · {[m.capture.source.app_name, m.capture.source.title].filter(Boolean).join(" — ")}</dd>
                  <dt className="text-muted">Obszar</dt><dd>{m.capture.crop ? "zaznaczony fragment" : "cały obraz"}</dd>
                </>
              )}
              <dt className="text-muted">Audio</dt><dd>{m.audio.file ? `${m.audio.file} (Opus ${m.audio.bitrate / 1000} kb/s)` : "usunięte po transkrypcji"}</dd>
              <dt className="text-muted">Silnik</dt><dd>{m.transcription.engine} · {m.transcription.model} · {m.transcription.language}</dd>
              <dt className="text-muted">Języki wykryte</dt><dd>{m.transcription.detected_languages.join(", ") || "—"}</dd>
              <dt className="text-muted">Fragmenty</dt><dd>{m.transcription.chunks_done}/{m.transcription.chunks_total}{m.transcription.chunks_failed ? `, błędy: ${m.transcription.chunks_failed}` : ""}</dd>
              <dt className="text-muted">Folder</dt><dd className="break-all">{path}</dd>
            </dl>
            <div className="mt-3"><Button size="sm" onClick={() => act(() => api.regenerateDocuments(path), "Dokumenty wygenerowane ponownie")}>Wygeneruj dokumenty ponownie</Button></div>
          </Card>
        </div>
      )}

      <Modal
        open={confirmDelete !== null}
        onClose={() => setConfirmDelete(null)}
        title={`Usunąć slajd ${confirmDelete}?`}
        footer={<>
          <Button variant="ghost" onClick={() => setConfirmDelete(null)}>Anuluj</Button>
          <Button variant="danger" loading={busy} onClick={() => act(() => api.deleteSlide(path, confirmDelete!).then(() => { setConfirmDelete(null); setSel(Math.max(0, sel - 1)); }), "Slajd usunięty")}>Usuń</Button>
        </>}
      >
        <p className="text-muted">Plik obrazu zostanie usunięty z folderu. Czas jego wyświetlania (i przypisane wypowiedzi) trafi do poprzedniego slajdu – transkrypcja nie zostanie utracona.</p>
      </Modal>

      <Modal
        open={retx}
        onClose={() => setRetx(false)}
        title="Transkrybuj ponownie"
        footer={<>
          <Button variant="ghost" onClick={() => setRetx(false)}>Anuluj</Button>
          <Button
            variant="primary"
            disabled={!models.find((x) => x.info.id === retxModel)?.installed}
            onClick={() => act(() => api.retranscribe(path, retxModel, retxLang).then(() => setRetx(false)), "Rozpoczęto transkrypcję")}
          >
            Rozpocznij
          </Button>
        </>}
      >
        <p className="text-muted mb-3">Całe nagranie zostanie przetworzone od nowa z pliku audio. Obecna transkrypcja zostanie zastąpiona po zakończeniu.</p>
        <div className="grid grid-cols-2 gap-3">
          <Select value={retxModel} onChange={setRetxModel} options={models.map((x) => ({ value: x.info.id, label: `${x.info.label}${x.installed ? "" : " – nie pobrano"}` }))} />
          <Select value={retxLang} onChange={setRetxLang} options={LANGUAGES.map((l) => ({ value: l.id, label: l.label }))} />
        </div>
      </Modal>

      <Modal
        open={zipOpen}
        onClose={() => setZipOpen(false)}
        title="Eksport do ZIP"
        footer={<>
          <Button variant="ghost" onClick={() => setZipOpen(false)}>Anuluj</Button>
          <Button
            variant="primary"
            loading={busy}
            onClick={async () => {
              const dest = await saveDialog({ defaultPath: `${m.lecture.folder}.zip`, filters: [{ name: "ZIP", extensions: ["zip"] }] });
              if (!dest) return;
              await act(async () => {
                const n = await api.exportZip(path, dest, zipAudio);
                setZipOpen(false);
                toast(`Wyeksportowano ${n} plików.`, "success");
              });
            }}
          >
            Zapisz ZIP…
          </Button>
        </>}
      >
        <Toggle checked={zipAudio} onChange={setZipAudio} label="Dołącz nagranie audio" description="Bez audio archiwum jest znacznie mniejsze; do pracy z agentem AI wystarczą slajdy i transkrypcja." />
      </Modal>
    </div>
  );
}

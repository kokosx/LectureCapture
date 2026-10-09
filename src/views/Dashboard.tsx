import { useEffect, useMemo, useState } from "react";
import { Plus, HardDrive, Cpu, Clock, Layers, BookOpen, Radio, RefreshCw, Folder, FolderPlus, Pencil, Trash2, Check, X } from "lucide-react";
import { api, fileSrc, LectureSummary, ModelStatus, SubjectInfo, SystemInfo } from "../lib/api";
import { errorText, fmtBytes, fmtDate, fmtDuration, plural, transcriptionLabel } from "../lib/format";
import { Badge, Button, Card, Empty, Input, cx, useToast } from "../components/ui";
import type { Nav } from "../App";

/** Library filter: all lectures, lectures without a subject, or one subject. */
type Filter = { kind: "all" } | { kind: "none" } | { kind: "subject"; name: string };
let lastFilter: Filter = { kind: "all" };

function LectureCard({ l, nav, showSubject }: { l: LectureSummary; nav: Nav; showSubject?: boolean }) {
  return (
    <button
      onClick={() => nav.go({ name: "lecture", path: l.path })}
      className="text-left bg-panel border border-line rounded-xl overflow-hidden hover:border-accent/50 hover:shadow-md transition group"
    >
      <div className="aspect-video bg-panel-2 overflow-hidden border-b border-line">
        {l.thumbnail ? (
          <img src={fileSrc(l.thumbnail)} className="w-full h-full object-cover group-hover:scale-[1.02] transition-transform" alt="" />
        ) : (
          <div className="w-full h-full flex items-center justify-center text-subtle"><Layers size={22} /></div>
        )}
      </div>
      <div className="p-3">
        {showSubject && l.subject && <div className="text-[11px] text-accent font-medium truncate mb-0.5">{l.subject}</div>}
        <div className="font-semibold truncate">{l.title}</div>
        <div className="text-muted text-[12px] mt-0.5">{fmtDate(l.started_at)}</div>
        <div className="flex flex-wrap items-center gap-1.5 mt-2">
          <Badge><Clock size={11} /> {fmtDuration(l.duration_ms)}</Badge>
          <Badge><Layers size={11} /> {l.slides}</Badge>
          <Badge tone={transcriptionTone(l.transcription)}>{transcriptionLabel[l.transcription]}</Badge>
          {l.status === "recording" && <Badge tone="red">nagrywanie</Badge>}
          {l.status === "recovered" && <Badge tone="yellow">odzyskany</Badge>}
        </div>
      </div>
    </button>
  );
}

function Chip({ active, onClick, children }: { active: boolean; onClick: () => void; children: React.ReactNode }) {
  return (
    <button
      onClick={onClick}
      className={cx(
        "inline-flex items-center gap-1.5 h-7 px-2.5 rounded-full border text-[12.5px] font-medium transition-colors",
        active ? "bg-accent/10 border-accent/40 text-accent" : "bg-panel border-line text-muted hover:text-fg hover:bg-panel-2",
      )}
    >
      {children}
    </button>
  );
}

export function transcriptionTone(s: string): "green" | "yellow" | "red" | "neutral" | "blue" {
  return s === "completed" ? "green" : s === "failed" ? "red" : s === "partial" ? "yellow" : s === "running" || s === "pending" ? "blue" : "neutral";
}

export function Dashboard({ nav, recording }: { nav: Nav; recording: boolean }) {
  const toast = useToast();
  const [lectures, setLectures] = useState<LectureSummary[] | null>(null);
  const [subjects, setSubjects] = useState<SubjectInfo[]>([]);
  const [models, setModels] = useState<ModelStatus[]>([]);
  const [info, setInfo] = useState<SystemInfo | null>(null);
  const [filter, setFilterState] = useState<Filter>(lastFilter);
  const [newSubject, setNewSubject] = useState<string | null>(null);
  const [renaming, setRenaming] = useState<string | null>(null);

  const setFilter = (f: Filter) => {
    lastFilter = f;
    setFilterState(f);
    setRenaming(null);
  };

  const load = () => {
    api.listLectures().then(setLectures).catch(() => setLectures([]));
    api.listSubjects().then(setSubjects).catch(() => setSubjects([]));
    api.modelsStatus().then(setModels);
    api.systemInfo().then(setInfo);
  };
  useEffect(load, []);

  // a subject that disappeared (renamed/removed outside the app) → back to all
  useEffect(() => {
    if (filter.kind === "subject" && subjects.length && !subjects.some((s) => s.name === filter.name)) setFilter({ kind: "all" });
  }, [subjects]); // eslint-disable-line react-hooks/exhaustive-deps

  const withoutSubject = lectures?.filter((l) => !l.subject) ?? [];
  const visible = useMemo(() => {
    if (!lectures) return null;
    if (filter.kind === "none") return lectures.filter((l) => !l.subject);
    if (filter.kind === "subject") return lectures.filter((l) => l.subject === filter.name);
    return lectures;
  }, [lectures, filter]);
  const currentSubject = filter.kind === "subject" ? subjects.find((s) => s.name === filter.name) : undefined;

  const run = async (fn: () => Promise<unknown>, ok?: string) => {
    try {
      await fn();
      if (ok) toast(ok, "success");
      return true;
    } catch (e) {
      toast(errorText(e), "error");
      return false;
    }
  };

  const createSubject = async () => {
    const name = newSubject?.trim();
    if (!name) return setNewSubject(null);
    let created: SubjectInfo | null = null;
    if (await run(async () => { created = await api.createSubject(name); })) {
      setNewSubject(null);
      load();
      if (created) setFilter({ kind: "subject", name: (created as SubjectInfo).name });
    }
  };

  const newLecture = () => nav.go({ name: "new", subject: filter.kind === "subject" ? filter.name : filter.kind === "none" ? "" : undefined });

  const installed = models.filter((m) => m.installed);
  const selected = models.find((m) => m.info.id === nav.settings.transcription.model);

  return (
    <div className="max-w-[1100px] mx-auto px-8 pb-10">
      <div className="flex items-end justify-between mb-6">
        <div>
          <h1 className="text-[22px] font-semibold tracking-tight">Wykłady</h1>
          <p className="text-muted mt-0.5">Slajdy, transkrypcja i prompt do nauki – w jednym folderze.</p>
        </div>
        {recording ? (
          <Button variant="rec" size="lg" icon={<Radio size={16} />} onClick={() => nav.go({ name: "recording" })}>Trwa nagrywanie</Button>
        ) : (
          <Button variant="primary" size="lg" icon={<Plus size={16} />} onClick={newLecture}>
            {currentSubject ? "Nowy wykład w przedmiocie" : "Nowy wykład"}
          </Button>
        )}
      </div>

      <div className="grid grid-cols-3 gap-3 mb-6">
        <Card>
          <div className="flex items-center gap-2 text-muted text-[12px] font-medium mb-2"><Cpu size={14} /> Model Whisper</div>
          <div className="font-semibold">{selected?.info.label ?? nav.settings.transcription.model}</div>
          <div className="mt-1.5">
            {selected?.installed ? <Badge tone="green">pobrany</Badge> : (
              <button onClick={() => nav.go({ name: "models" })}><Badge tone="yellow">nie pobrano – kliknij, aby pobrać</Badge></button>
            )}
            <span className="text-subtle text-[12px] ml-2">{installed.length} z {models.length} modeli na dysku</span>
          </div>
        </Card>
        <Card>
          <div className="flex items-center gap-2 text-muted text-[12px] font-medium mb-2"><HardDrive size={14} /> Wolne miejsce</div>
          <div className="font-semibold">{fmtBytes(info?.free_bytes)}</div>
          <div className="text-subtle text-[12px] mt-1.5 truncate" title={info?.lectures_root}>{info?.lectures_root}</div>
        </Card>
        <Card>
          <div className="flex items-center gap-2 text-muted text-[12px] font-medium mb-2"><BookOpen size={14} /> Biblioteka</div>
          <div className="font-semibold">{lectures ? plural(lectures.length, "wykład", "wykłady", "wykładów") : "…"}</div>
          <div className="text-subtle text-[12px] mt-1.5">{fmtBytes(lectures?.reduce((a, l) => a + l.size_bytes, 0) ?? 0)} łącznie</div>
        </Card>
      </div>

      <div className="flex items-center justify-between mb-3">
        <h2 className="text-[12px] font-semibold uppercase tracking-wide text-muted">Przedmioty</h2>
        <Button variant="ghost" size="sm" icon={<RefreshCw size={13} />} onClick={load}>Odśwież</Button>
      </div>
      <div className="flex flex-wrap items-center gap-1.5 mb-5">
        <Chip active={filter.kind === "all"} onClick={() => setFilter({ kind: "all" })}>
          Wszystkie <span className="text-subtle">{lectures?.length ?? 0}</span>
        </Chip>
        {subjects.map((s) => (
          <Chip key={s.name} active={filter.kind === "subject" && filter.name === s.name} onClick={() => setFilter({ kind: "subject", name: s.name })}>
            <Folder size={12} /> {s.name} <span className="text-subtle">{s.lectures}</span>
          </Chip>
        ))}
        {withoutSubject.length > 0 && subjects.length > 0 && (
          <Chip active={filter.kind === "none"} onClick={() => setFilter({ kind: "none" })}>
            Bez przedmiotu <span className="text-subtle">{withoutSubject.length}</span>
          </Chip>
        )}
        {newSubject === null ? (
          <Button size="sm" variant="ghost" icon={<FolderPlus size={13} />} onClick={() => setNewSubject("")}>Nowy przedmiot</Button>
        ) : (
          <form className="flex items-center gap-1" onSubmit={(e) => { e.preventDefault(); createSubject(); }}>
            <Input
              autoFocus
              placeholder="np. Analiza matematyczna"
              value={newSubject}
              onChange={(e) => setNewSubject(e.target.value)}
              onKeyDown={(e) => e.key === "Escape" && setNewSubject(null)}
              className="h-7 w-56 text-[12.5px]"
            />
            <Button size="sm" type="submit" icon={<Check size={13} />} />
            <Button size="sm" variant="ghost" icon={<X size={13} />} onClick={() => setNewSubject(null)} />
          </form>
        )}
      </div>

      {currentSubject && (
        <div className="flex items-center gap-2 mb-3">
          {renaming !== null ? (
            <form
              className="flex items-center gap-1"
              onSubmit={async (e) => {
                e.preventDefault();
                const target = renaming.trim();
                if (await run(() => api.renameSubject(currentSubject.name, target), "Zmieniono nazwę przedmiotu")) {
                  setFilter({ kind: "subject", name: target });
                  load();
                }
              }}
            >
              <Input autoFocus value={renaming} onChange={(e) => setRenaming(e.target.value)} onKeyDown={(e) => e.key === "Escape" && setRenaming(null)} className="h-8 w-72 font-semibold" />
              <Button size="sm" type="submit" icon={<Check size={13} />} />
              <Button size="sm" variant="ghost" icon={<X size={13} />} onClick={() => setRenaming(null)} />
            </form>
          ) : (
            <>
              <h2 className="text-[16px] font-semibold flex items-center gap-2"><Folder size={16} className="text-accent" /> {currentSubject.name}</h2>
              <span className="text-muted text-[12.5px]">{plural(currentSubject.lectures, "wykład", "wykłady", "wykładów")}</span>
              <Button size="sm" variant="ghost" icon={<Pencil size={12} />} onClick={() => setRenaming(currentSubject.name)}>Zmień nazwę</Button>
              {currentSubject.lectures === 0 && (
                <Button
                  size="sm"
                  variant="ghost"
                  icon={<Trash2 size={12} />}
                  onClick={async () => {
                    if (await run(() => api.deleteSubject(currentSubject.name), "Usunięto pusty przedmiot")) {
                      setFilter({ kind: "all" });
                      load();
                    }
                  }}
                >
                  Usuń
                </Button>
              )}
            </>
          )}
        </div>
      )}

      {lectures && lectures.length === 0 && (
        <Card>
          <Empty icon={<BookOpen size={28} />} title="Brak nagranych wykładów">
            Utwórz przedmiot (np. „Analiza matematyczna”), kliknij „Nowy wykład”, wybierz okno Teams i rozpocznij nagrywanie.
            Po zakończeniu znajdziesz tu slajdy, transkrypcję i PROMPT.md.
          </Empty>
        </Card>
      )}
      {lectures && lectures.length > 0 && visible && visible.length === 0 && (
        <Card>
          <Empty icon={<Folder size={28} />} title="Brak wykładów w tym przedmiocie">
            Kliknij „Nowy wykład w przedmiocie”, aby nagrać pierwszy – albo przenieś istniejący wykład z jego widoku szczegółów.
          </Empty>
        </Card>
      )}
      <div className="grid grid-cols-[repeat(auto-fill,minmax(250px,1fr))] gap-3">
        {visible?.map((l) => <LectureCard key={l.path} l={l} nav={nav} showSubject={filter.kind === "all"} />)}
      </div>
    </div>
  );
}

import { useEffect, useState } from "react";
import { Plus, HardDrive, Cpu, Clock, Layers, BookOpen, Radio, RefreshCw } from "lucide-react";
import { api, fileSrc, LectureSummary, ModelStatus, SystemInfo } from "../lib/api";
import { fmtBytes, fmtDate, fmtDuration, plural, transcriptionLabel } from "../lib/format";
import { Badge, Button, Card, Empty } from "../components/ui";
import type { Nav } from "../App";

export function transcriptionTone(s: string): "green" | "yellow" | "red" | "neutral" | "blue" {
  return s === "completed" ? "green" : s === "failed" ? "red" : s === "partial" ? "yellow" : s === "running" || s === "pending" ? "blue" : "neutral";
}

export function Dashboard({ nav, recording }: { nav: Nav; recording: boolean }) {
  const [lectures, setLectures] = useState<LectureSummary[] | null>(null);
  const [models, setModels] = useState<ModelStatus[]>([]);
  const [info, setInfo] = useState<SystemInfo | null>(null);

  const load = () => {
    api.listLectures().then(setLectures).catch(() => setLectures([]));
    api.modelsStatus().then(setModels);
    api.systemInfo().then(setInfo);
  };
  useEffect(load, []);

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
          <Button variant="primary" size="lg" icon={<Plus size={16} />} onClick={() => nav.go({ name: "new" })}>Nowy wykład</Button>
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
        <h2 className="text-[12px] font-semibold uppercase tracking-wide text-muted">Ostatnie wykłady</h2>
        <Button variant="ghost" size="sm" icon={<RefreshCw size={13} />} onClick={load}>Odśwież</Button>
      </div>
      {lectures && lectures.length === 0 && (
        <Card>
          <Empty icon={<BookOpen size={28} />} title="Brak nagranych wykładów">
            Kliknij „Nowy wykład”, wybierz okno Teams i rozpocznij nagrywanie. Po zakończeniu znajdziesz tu slajdy, transkrypcję i PROMPT.md.
          </Empty>
        </Card>
      )}
      <div className="grid grid-cols-[repeat(auto-fill,minmax(250px,1fr))] gap-3">
        {lectures?.map((l) => (
          <button
            key={l.path}
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
        ))}
      </div>
    </div>
  );
}

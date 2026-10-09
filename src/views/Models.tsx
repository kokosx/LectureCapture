import { useEffect, useState } from "react";
import { Download, Trash2, X, CheckCircle2 } from "lucide-react";
import { api, DownloadStatus, ModelStatus } from "../lib/api";
import { fmtBytes, errorText } from "../lib/format";
import { Badge, Button, Card, Progress, useToast } from "../components/ui";

export function useModels() {
  const [models, setModels] = useState<ModelStatus[]>([]);
  const [downloads, setDownloads] = useState<Record<string, DownloadStatus>>({});
  useEffect(() => {
    let alive = true;
    const tick = async () => {
      const [m, d] = await Promise.all([api.modelsStatus(), api.downloadStatus()]);
      if (alive) {
        setModels(m);
        setDownloads(d);
      }
    };
    tick();
    const t = setInterval(tick, 700);
    return () => {
      alive = false;
      clearInterval(t);
    };
  }, []);
  return { models, downloads };
}

export function ModelRow({ m, d }: { m: ModelStatus; d?: DownloadStatus }) {
  const toast = useToast();
  const running = d?.state === "running";
  return (
    <div className="flex items-center gap-4 py-3 border-b border-line last:border-b-0">
      <div className="flex-1 min-w-0">
        <div className="flex items-center gap-2">
          <span className="font-semibold">{m.info.label}</span>
          {m.info.default && <Badge tone="accent">domyślny</Badge>}
          {m.installed && <Badge tone="green"><CheckCircle2 size={11} /> pobrany{m.verified ? " · zweryfikowany" : ""}</Badge>}
        </div>
        <div className="text-muted text-[12.5px] mt-0.5">{m.info.description}</div>
        {running && (
          <div className="mt-2 flex items-center gap-3">
            <Progress value={d!.total ? d!.downloaded / d!.total : 0} className="flex-1" />
            <span className="text-[12px] text-muted tabular-nums w-28 text-right">{fmtBytes(d!.downloaded)} / {fmtBytes(d!.total)}</span>
          </div>
        )}
        {d?.state === "failed" && <div className="text-red-500 text-[12px] mt-1 selectable">{d.error}</div>}
      </div>
      <div className="text-subtle text-[12px] w-16 text-right">{fmtBytes(m.info.size)}</div>
      <div className="w-32 flex justify-end">
        {running ? (
          <Button size="sm" icon={<X size={13} />} onClick={() => api.cancelDownload(m.info.id)}>Anuluj</Button>
        ) : m.installed ? (
          <Button size="sm" variant="ghost" icon={<Trash2 size={13} />} onClick={() => api.deleteModel(m.info.id).catch((e) => toast(errorText(e), "error"))}>Usuń</Button>
        ) : (
          <Button size="sm" variant="primary" icon={<Download size={13} />} onClick={() => api.downloadModel(m.info.id).catch((e) => toast(errorText(e), "error"))}>Pobierz</Button>
        )}
      </div>
    </div>
  );
}

export function ModelsView() {
  const { models, downloads } = useModels();
  return (
    <div className="max-w-[860px] mx-auto px-8 pb-10">
      <h1 className="text-[22px] font-semibold tracking-tight">Modele Whisper</h1>
      <p className="text-muted mt-0.5 mb-6">
        Transkrypcja działa lokalnie (whisper.cpp, akceleracja Metal na Apple Silicon). Modele pochodzą z oficjalnego repozytorium
        whisper.cpp na Hugging Face; każdy plik jest weryfikowany sumą SHA-256 przed użyciem.
      </p>
      <Card>
        {models.map((m) => <ModelRow key={m.info.id} m={m} d={downloads[m.info.id]} />)}
      </Card>
      <p className="text-subtle text-[12px] mt-4 leading-relaxed">
        Wskazówka: dla języka polskiego najlepsze wyniki daje „Large v3 Turbo”, a na MacBooku z czipem M działa szybciej niż
        w czasie rzeczywistym. „Base” to rozsądny kompromis na słabszych komputerach. Licencja modeli: MIT (OpenAI Whisper).
      </p>
    </div>
  );
}

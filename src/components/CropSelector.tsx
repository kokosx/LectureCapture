import { useRef, useState } from "react";
import type { NormRect } from "../lib/api";

/** Preview image on which the user drags the presentation area. */
export function CropSelector({ src, crop, onChange, nativeW, nativeH }: {
  src: string; crop: NormRect | null; onChange: (r: NormRect | null) => void; nativeW: number; nativeH: number;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [drag, setDrag] = useState<{ x0: number; y0: number; x1: number; y1: number } | null>(null);

  const pos = (e: React.MouseEvent) => {
    const r = ref.current!.getBoundingClientRect();
    return {
      x: Math.max(0, Math.min(1, (e.clientX - r.left) / r.width)),
      y: Math.max(0, Math.min(1, (e.clientY - r.top) / r.height)),
    };
  };

  const live = drag
    ? { x: Math.min(drag.x0, drag.x1), y: Math.min(drag.y0, drag.y1), w: Math.abs(drag.x1 - drag.x0), h: Math.abs(drag.y1 - drag.y0) }
    : crop;

  return (
    <div className="relative select-none">
      <div
        ref={ref}
        className="relative cursor-crosshair rounded-lg overflow-hidden border border-line bg-black"
        onMouseDown={(e) => {
          const p = pos(e);
          setDrag({ x0: p.x, y0: p.y, x1: p.x, y1: p.y });
        }}
        onMouseMove={(e) => {
          if (!drag) return;
          const p = pos(e);
          setDrag({ ...drag, x1: p.x, y1: p.y });
        }}
        onMouseUp={() => {
          if (!drag) return;
          const r = { x: Math.min(drag.x0, drag.x1), y: Math.min(drag.y0, drag.y1), w: Math.abs(drag.x1 - drag.x0), h: Math.abs(drag.y1 - drag.y0) };
          setDrag(null);
          onChange(r.w > 0.03 && r.h > 0.03 ? r : null);
        }}
        onMouseLeave={() => drag && setDrag(null)}
      >
        <img src={src} draggable={false} className="w-full block pointer-events-none" alt="Podgląd źródła" />
        {live && (
          <>
            <div className="absolute inset-0 pointer-events-none" style={{
              background: "rgba(0,0,0,0.55)",
              clipPath: `polygon(0 0, 100% 0, 100% 100%, 0 100%, 0 0, ${live.x * 100}% ${live.y * 100}%, ${live.x * 100}% ${(live.y + live.h) * 100}%, ${(live.x + live.w) * 100}% ${(live.y + live.h) * 100}%, ${(live.x + live.w) * 100}% ${live.y * 100}%, ${live.x * 100}% ${live.y * 100}%)`,
            }} />
            <div
              className="absolute border-2 border-accent rounded-sm pointer-events-none shadow-[0_0_0_1px_rgba(255,255,255,0.4)]"
              style={{ left: `${live.x * 100}%`, top: `${live.y * 100}%`, width: `${live.w * 100}%`, height: `${live.h * 100}%` }}
            >
              <span className="absolute -top-6 left-0 text-[11px] bg-accent text-white px-1.5 py-0.5 rounded tabular-nums">
                {Math.round(live.w * nativeW)} × {Math.round(live.h * nativeH)} px
              </span>
            </div>
          </>
        )}
      </div>
    </div>
  );
}

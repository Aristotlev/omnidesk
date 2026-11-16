import { useEffect, useRef, useState, type CSSProperties, type ReactNode, type RefObject } from "react";
import { Laptop, LoaderCircle, Monitor, Smartphone, Tablet } from "lucide-react";
import { claimStream, onFrame } from "./api";
import type { Device, Kind, Os, StreamMode } from "./types";

export function Spinner({ size = 14 }: { size?: number }) {
  return <LoaderCircle size={size} className="spin" />;
}

export function Card(p: { title?: string; sub?: string; icon?: ReactNode; end?: ReactNode; children: ReactNode; className?: string }) {
  return (
    <section className={`card ${p.className ?? ""}`}>
      {(p.title || p.end) && (
        <div className="card-h">
          {p.icon && <div className="icon-chip">{p.icon}</div>}
          <div>
            {p.title && <h3>{p.title}</h3>}
            {p.sub && <div className="sub">{p.sub}</div>}
          </div>
          {p.end && <div className="end">{p.end}</div>}
        </div>
      )}
      {p.children}
    </section>
  );
}

const KIND_ICON: Record<Kind, typeof Monitor> = { desktop: Monitor, laptop: Laptop, phone: Smartphone, tablet: Tablet };
const OS_LABEL: Record<Os, string> = { windows: "Windows", macos: "Mac", linux: "Linux", ios: "iPhone / iPad", android: "Android" };
const KIND_LABEL: Record<Kind, string> = { desktop: "desktop", laptop: "laptop", phone: "phone", tablet: "tablet" };

export function DeviceIcon({ kind, size = 16 }: { kind: Kind; size?: number }) {
  const Icon = KIND_ICON[kind];
  return <Icon size={size} />;
}

/** "Windows desktop", "Mac laptop". */
export function describe(d: { os: Os; kind: Kind }): string {
  return d.os === "ios" || d.os === "android" ? OS_LABEL[d.os] : `${OS_LABEL[d.os]} ${KIND_LABEL[d.kind]}`;
}

export function StatusPill({ d }: { d: Device }) {
  if (!d.paired) return <span className="pill info">On this network</span>;
  return d.online ? <span className="pill ok">Online</span> : <span className="pill">Offline</span>;
}

export function Toggle(p: { on: boolean; onChange: (on: boolean) => void; label: string; hint?: string; disabled?: boolean }) {
  return (
    <label className={`toggle ${p.disabled ? "disabled" : ""}`}>
      <span className="toggle-text">
        <span className="strong">{p.label}</span>
        {p.hint && <span className="muted small">{p.hint}</span>}
      </span>
      <input type="checkbox" role="switch" checked={p.on} disabled={p.disabled} onChange={(e) => p.onChange(e.target.checked)} />
      <span className="switch" aria-hidden />
    </label>
  );
}

/** Six boxes that read as one code. */
export function Code({ code }: { code: string }) {
  return (
    <div className="code" aria-label={`Pairing code ${code.split("").join(" ")}`}>
      {code.split("").map((c, i) => (
        <span key={i} className={i === 3 ? "gap" : ""}>{c}</span>
      ))}
    </div>
  );
}

/**
 * Live picture of one screen of another device. Starts the stream when shown and stops it
 * when removed. `mode` picks wall-thumbnail or full size.
 */
export function ScreenCanvas(p: { device: string; screen: number; mode: StreamMode; innerRef?: RefObject<HTMLCanvasElement | null>; onLive?: () => void; style?: CSSProperties }) {
  const own = useRef<HTMLCanvasElement | null>(null);
  const ref = p.innerRef ?? own;
  const [live, setLive] = useState(false);
  const { onLive } = p;

  useEffect(() => {
    let alive = true;
    let arrived = 0;
    let drawn = 0;
    setLive(false);
    const off = onFrame(p.device, p.screen, (frame) => {
      const n = ++arrived;
      const img = new Image();
      img.onload = () => {
        const canvas = ref.current;
        // A frame that finished loading after a newer one was drawn is skipped.
        if (!alive || !canvas || n < drawn) return;
        if (canvas.width !== img.naturalWidth || canvas.height !== img.naturalHeight) {
          canvas.width = img.naturalWidth;
          canvas.height = img.naturalHeight;
        }
        canvas.getContext("2d")?.drawImage(img, 0, 0);
        if (drawn === 0) onLive?.();
        drawn = n;
        setLive(true);
      };
      img.src = frame.url;
    });
    const release = claimStream(p.device, p.screen, p.mode);
    return () => {
      alive = false;
      off();
      release();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [p.device, p.screen, p.mode]);

  return (
    <>
      <canvas ref={ref} className={`screen ${live ? "live" : ""}`} width={16} height={9} style={p.style} />
      {!live && <div className="screen-wait"><Spinner size={18} /></div>}
    </>
  );
}

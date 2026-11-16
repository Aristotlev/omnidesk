import { useCallback, useEffect, useRef, useState } from "react";
import { ArrowLeft, Eye, MousePointer2 } from "lucide-react";
import { api } from "../api";
import { DeviceIcon, ScreenCanvas } from "../components";
import type { Device, InputEvent, MouseButton } from "../types";

const BUTTONS: MouseButton[] = ["left", "middle", "right", "back", "forward"];
const clamp01 = (n: number) => Math.min(1, Math.max(0, n));
/** Browser wheel deltas (pixels or lines) to wheel clicks. */
const wheelSteps = (delta: number) => (delta === 0 ? 0 : Math.sign(delta) * Math.max(1, Math.round(Math.abs(delta) / 100)));

interface Props {
  device: Device;
  screen: number;
  error?: string;
  onScreen: (screen: number) => void;
  onClose: () => void;
}

/** One screen of another device, full size. Keyboard and pointer go to that device. */
export default function Control({ device, screen, error, onScreen, onClose }: Props) {
  const canControl = !!device.capabilities?.control;
  const [controlling, setControlling] = useState(canControl);
  const active = controlling && canControl && device.online;
  const canvas = useRef<HTMLCanvasElement | null>(null);
  const stage = useRef<HTMLDivElement | null>(null);
  const held = useRef(new Map<string, string>());
  const pendingMove = useRef<{ x: number; y: number } | null>(null);
  const raf = useRef(0);
  const info = device.screens.find((s) => s.index === screen);
  const [fit, setFit] = useState<{ width: number; height: number } | null>(null);

  // The picture fills the window, keeping the other screen's shape, also when it is smaller.
  useEffect(() => {
    const el = stage.current;
    if (!el || !info) return;
    const measure = () => {
      const pad = 20;
      const w = Math.max(0, el.clientWidth - pad);
      const h = Math.max(0, el.clientHeight - pad);
      const scale = Math.min(w / info.width, h / info.height);
      setFit({ width: Math.floor(info.width * scale), height: Math.floor(info.height * scale) });
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, [info?.width, info?.height]);

  const send = useCallback((event: InputEvent) => void api.sendInput(device.id, event).catch(() => {}), [device.id]);

  const releaseKeys = useCallback(() => {
    held.current.forEach((key, code) => send({ key: { code, key, down: false } }));
    held.current.clear();
  }, [send]);

  // Leaving the view (or switching to watch-only) must never leave a key stuck down over there.
  useEffect(() => releaseKeys, [releaseKeys]);
  useEffect(() => {
    if (!active) releaseKeys();
    else stage.current?.focus();
  }, [active, releaseKeys]);

  const position = (e: { clientX: number; clientY: number }) => {
    const r = canvas.current?.getBoundingClientRect();
    if (!r || r.width === 0) return null;
    return { x: clamp01((e.clientX - r.left) / r.width), y: clamp01((e.clientY - r.top) / r.height) };
  };

  const flushMove = () => {
    raf.current = 0;
    const p = pendingMove.current;
    pendingMove.current = null;
    if (p) send({ pointer_abs: { screen, ...p } });
  };

  const onPointerMove = (e: React.PointerEvent) => {
    // Only while over the picture (or dragging, when the pointer is captured).
    if (!active || (e.target !== canvas.current && !canvas.current?.hasPointerCapture(e.pointerId))) return;
    pendingMove.current = position(e);
    // At most one position per painted frame: the newest.
    if (!raf.current) raf.current = requestAnimationFrame(flushMove);
  };

  const onPointerDown = (e: React.PointerEvent) => {
    if (!active) return;
    stage.current?.focus();
    if (e.target !== canvas.current) return;
    canvas.current?.setPointerCapture(e.pointerId);
    const p = position(e);
    const button = BUTTONS[e.button];
    if (!p || !button) return;
    pendingMove.current = null;
    send({ pointer_abs: { screen, ...p } });
    send({ button: { button, down: true } });
  };

  const onPointerUp = (e: React.PointerEvent) => {
    const button = BUTTONS[e.button];
    if (active && button) send({ button: { button, down: false } });
  };

  const onWheel = (e: React.WheelEvent) => {
    if (!active) return;
    const dx = wheelSteps(e.deltaX);
    const dy = wheelSteps(e.deltaY);
    if (dx || dy) send({ scroll: { dx, dy } });
  };

  const onKey = (down: boolean) => (e: React.KeyboardEvent) => {
    if (!active) return;
    e.preventDefault();
    if (down) held.current.set(e.code, e.key);
    else held.current.delete(e.code);
    send({ key: { code: e.code, key: e.key, down } });
  };

  const problem = !device.online ? `${device.name} went offline.` : error;

  return (
    <div className="control">
      <div className="control-bar">
        <button className="btn" onClick={onClose}><ArrowLeft size={15} />All screens</button>
        <span className="control-name"><DeviceIcon kind={device.kind} /><span className="strong ellipsis">{device.name}</span></span>
        {device.screens.length > 1 && (
          <div className="seg">
            {device.screens.map((s) => (
              <button key={s.index} className={s.index === screen ? "on" : ""} onClick={() => onScreen(s.index)}>{s.name || `Display ${s.index + 1}`}</button>
            ))}
          </div>
        )}
        <span className="grow" />
        {canControl ? (
          <div className="seg">
            <button className={controlling ? "on" : ""} onClick={() => setControlling(true)}><MousePointer2 size={14} />Control</button>
            <button className={!controlling ? "on" : ""} onClick={() => setControlling(false)}><Eye size={14} />Watch only</button>
          </div>
        ) : (
          <span className="muted small">Control is turned off on {device.name}</span>
        )}
      </div>
      <div
        ref={stage}
        className={`control-stage ${active ? "active" : ""}`}
        tabIndex={0}
        onKeyDown={onKey(true)}
        onKeyUp={onKey(false)}
        onBlur={releaseKeys}
        onPointerMove={onPointerMove}
        onPointerDown={onPointerDown}
        onPointerUp={onPointerUp}
        onWheel={onWheel}
        onContextMenu={(e) => e.preventDefault()}
      >
        {problem ? <div className="tile-msg">{problem}</div> : <ScreenCanvas device={device.id} screen={screen} mode="focus" innerRef={canvas} style={fit ?? undefined} />}
      </div>
      <div className="control-hint">
        {active ? <>Your keyboard and mouse now work on <b>{device.name}</b>. Click “All screens” to stop.</> : canControl ? "Watching only. Choose Control to use your keyboard and mouse there." : "Watching only."}
      </div>
    </div>
  );
}

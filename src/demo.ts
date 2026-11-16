// Sample devices for the browser preview (`npm run dev`). Nothing here runs in the real app.
import type { Device, FrameEvent, InputEvent, PairingOffer, PermissionKind, Permissions, Self, Settings, StreamMode } from "./types";

const caps = { view: "always" as const, control: true, grab: false, display_sink: false, display_source: false, clipboard: true };
const screen = (index: number, name: string, width: number, height: number, x = 0) => ({ index, name, x, y: 0, width, height, scale: 1, primary: index === 0 });

export const state = {
  autostart: true,
  settings: { name: null, allow_view: true, allow_control: true, share_clipboard: true } as Settings,
  devices: [
    { id: "a41c09e27fb35d10", name: "Studio PC", kind: "desktop", os: "windows", paired: true, online: true, nearby: true, address: "192.168.1.20", capabilities: caps, screens: [screen(0, "DELL U2723QE", 2560, 1440), screen(1, "LG 24MP", 1920, 1080, 2560)], app_version: "0.1.0" },
    { id: "5be2710c88d4f6a3", name: "Aristotle's MacBook Pro", kind: "laptop", os: "macos", paired: true, online: true, nearby: true, address: "192.168.1.31", capabilities: caps, screens: [screen(0, "Built-in Retina Display", 1512, 982)], app_version: "0.1.0" },
    { id: "0f93b6a1c27e4d58", name: "Media server", kind: "desktop", os: "linux", paired: true, online: false, nearby: false, address: "192.168.1.44", capabilities: null, screens: [], app_version: null },
    { id: "77d0e4c9a1b2f365", name: "Workshop laptop", kind: "laptop", os: "windows", paired: false, online: false, nearby: true, address: "192.168.1.52", capabilities: null, screens: [], app_version: null },
  ] as Device[],
};

const deviceListeners = new Set<(d: Device[]) => void>();
const eventListeners = new Map<string, Set<(p: unknown) => void>>();
const changed = () => deviceListeners.forEach((cb) => cb([...state.devices]));

export const self = async (): Promise<Self> => ({
  info: { id: "c3d87a10e55f2b94", name: state.settings.name ?? "Office desktop", kind: "desktop", os: "windows", screens: [screen(0, "Display 1", 2560, 1440)], capabilities: caps },
  settings: state.settings,
  address: "192.168.1.10",
  port: 47800,
});
export const devices = async () => [...state.devices];
export const beginPairing = async (): Promise<PairingOffer> => ({ code: "482916", seconds: 120 });

export async function pair(target: string, code: string): Promise<string> {
  await new Promise((r) => setTimeout(r, 700));
  if (code.replace(/\D/g, "") !== "123456") throw "The code does not match. Check the code on the other device and try again. (In this preview the code is 123456.)";
  const d = state.devices.find((x) => x.id === target);
  if (!d) throw "Could not reach the device at " + target;
  Object.assign(d, { paired: true, online: true, capabilities: caps, screens: [screen(0, "Display 1", 1920, 1080)], app_version: "0.1.0" });
  changed();
  return d.id;
}

export async function unpair(id: string) {
  const d = state.devices.find((x) => x.id === id);
  if (d) Object.assign(d, { paired: false, online: false, capabilities: null, screens: [] });
  state.devices = state.devices.filter((x) => x.paired || x.nearby);
  changed();
}

export async function setSettings(s: Settings) {
  state.settings = s;
}

export function onDevices(cb: (d: Device[]) => void) {
  deviceListeners.add(cb);
  return () => void deviceListeners.delete(cb);
}

export function onEvent(name: string, cb: (p: unknown) => void) {
  if (!eventListeners.has(name)) eventListeners.set(name, new Set());
  eventListeners.get(name)!.add(cb);
  return () => void eventListeners.get(name)?.delete(cb);
}

// ---- generated screens

const watching = new Map<string, { mode: StreamMode; seq: number; pointer: { x: number; y: number } }>();
let frameCb: ((f: FrameEvent) => void) | null = null;
let timer: number | undefined;
const lastUrl = new Map<string, string>();

function paint(device: Device, screenIndex: number, w: number, h: number, t: number, pointer: { x: number; y: number }): HTMLCanvasElement {
  const c = document.createElement("canvas");
  c.width = w;
  c.height = h;
  const g = c.getContext("2d")!;
  const hue = (parseInt(device.id.slice(0, 2), 16) + screenIndex * 70) % 360;
  const bg = g.createLinearGradient(0, 0, w, h);
  bg.addColorStop(0, `hsl(${hue} 55% 24%)`);
  bg.addColorStop(1, `hsl(${(hue + 60) % 360} 60% 12%)`);
  g.fillStyle = bg;
  g.fillRect(0, 0, w, h);
  // two windows and a taskbar
  const win = (x: number, y: number, ww: number, hh: number, title: string) => {
    g.fillStyle = "rgba(245,247,252,0.96)";
    g.beginPath();
    g.roundRect(x, y, ww, hh, w * 0.008);
    g.fill();
    g.fillStyle = "rgba(20,28,60,0.08)";
    g.fillRect(x, y, ww, h * 0.045);
    g.fillStyle = "#2a3358";
    g.font = `600 ${Math.round(h * 0.024)}px system-ui, sans-serif`;
    g.fillText(title, x + w * 0.012, y + h * 0.031);
    g.fillStyle = "rgba(42,51,88,0.16)";
    for (let i = 0; i < 6; i++) g.fillRect(x + w * 0.015, y + h * (0.075 + i * 0.045), ww * (0.5 + ((i * 37 + t) % 40) / 100), h * 0.014);
  };
  win(w * 0.06, h * 0.1, w * 0.5, h * 0.62, "Quarterly notes");
  win(w * 0.46, h * 0.26, w * 0.46, h * 0.56, device.os === "macos" ? "Terminal" : "Browser");
  g.fillStyle = "rgba(10,14,30,0.72)";
  g.fillRect(0, h * 0.94, w, h * 0.06);
  g.fillStyle = "#e8ebf5";
  g.font = `500 ${Math.round(h * 0.026)}px system-ui, sans-serif`;
  g.textAlign = "right";
  g.fillText(new Date().toLocaleTimeString(), w * 0.985, h * 0.98);
  g.textAlign = "left";
  // pointer
  const px = pointer.x * w;
  const py = pointer.y * h;
  g.fillStyle = "#fff";
  g.strokeStyle = "#111";
  g.lineWidth = Math.max(1, w / 900);
  g.beginPath();
  g.moveTo(px, py);
  g.lineTo(px, py + h * 0.035);
  g.lineTo(px + h * 0.01, py + h * 0.027);
  g.lineTo(px + h * 0.024, py + h * 0.026);
  g.closePath();
  g.fill();
  g.stroke();
  return c;
}

function tick() {
  watching.forEach((w, key) => {
    const [id, idx] = key.split("-");
    const device = state.devices.find((d) => d.id === id);
    const info = device?.screens[Number(idx)];
    if (!device || !info || !device.online) return;
    const width = w.mode === "focus" ? Math.min(1920, info.width) : 640;
    const height = Math.round((width * info.height) / info.width);
    w.seq += 1;
    paint(device, Number(idx), width, height, w.seq, w.pointer).toBlob(
      (blob) => {
        if (!blob || !watching.has(key)) return;
        const old = lastUrl.get(key);
        const url = URL.createObjectURL(blob);
        lastUrl.set(key, url);
        frameCb?.({ device: id, screen: Number(idx), seq: w.seq, width, height, url });
        if (old) setTimeout(() => URL.revokeObjectURL(old), 2000);
      },
      "image/jpeg",
      0.8,
    );
  });
}

export function onFrame(cb: (f: FrameEvent) => void) {
  frameCb = cb;
  timer = window.setInterval(tick, 500);
  return () => window.clearInterval(timer);
}

export async function watch(device: string, screenIndex: number, mode: StreamMode) {
  const key = `${device}-${screenIndex}`;
  watching.set(key, { mode, seq: watching.get(key)?.seq ?? 0, pointer: watching.get(key)?.pointer ?? { x: 0.5, y: 0.5 } });
  setTimeout(tick, 60);
}

export async function unwatch(device: string, screenIndex: number) {
  watching.delete(`${device}-${screenIndex}`);
}

export async function sendInput(device: string, event: InputEvent) {
  if ("pointer_abs" in event) {
    const w = watching.get(`${device}-${event.pointer_abs.screen}`);
    if (w) w.pointer = { x: event.pointer_abs.x, y: event.pointer_abs.y };
  }
}

// ---- macOS permissions (the preview acts like a Mac that has not allowed anything yet; add ?mac to the address)

const perms: Permissions = new URLSearchParams(window.location.search).has("mac") ? { screen: "missing", control: "missing" } : { screen: "not_needed", control: "not_needed" };
export const permissions = async (): Promise<Permissions> => ({ ...perms });
export async function requestPermission(kind: PermissionKind) {
  // In the preview, "allowing" takes effect after a short moment (screen: after reopening).
  if (kind === "control") setTimeout(() => (perms.control = "granted"), 1200);
}

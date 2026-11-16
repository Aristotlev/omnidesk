import type { Device, FrameEvent, InputEvent, PairingOffer, PermissionKind, Permissions, Self, Settings, StreamMode } from "./types";
import * as demo from "./demo";

/** True in a plain browser (`npm run dev`): the UI runs on sample devices. */
export const DEMO = !("__TAURI_INTERNALS__" in window);

async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const core = await import("@tauri-apps/api/core");
  return core.invoke<T>(cmd, args);
}

function listen<T>(name: string, cb: (payload: T) => void): () => void {
  const p = import("@tauri-apps/api/event").then((ev) => ev.listen<T>(name, (e) => cb(e.payload)));
  return () => void p.then((un) => un());
}

export const api = {
  self: (): Promise<Self> => (DEMO ? demo.self() : invoke("self_info")),
  devices: (): Promise<Device[]> => (DEMO ? demo.devices() : invoke("devices")),
  beginPairing: (): Promise<PairingOffer> => (DEMO ? demo.beginPairing() : invoke("begin_pairing")),
  cancelPairing: (): Promise<void> => (DEMO ? Promise.resolve() : invoke("cancel_pairing")),
  pair: (target: string, code: string): Promise<string> => (DEMO ? demo.pair(target, code) : invoke("pair", { target, code })),
  unpair: (device: string): Promise<void> => (DEMO ? demo.unpair(device) : invoke("unpair", { device })),
  setSettings: (settings: Settings): Promise<void> => (DEMO ? demo.setSettings(settings) : invoke("set_settings", { settings })),
  watch: (device: string, screen: number, mode: StreamMode): Promise<void> => (DEMO ? demo.watch(device, screen, mode) : invoke("watch", { device, screen, mode })),
  unwatch: (device: string, screen: number): Promise<void> => (DEMO ? demo.unwatch(device, screen) : invoke("unwatch", { device, screen })),
  sendInput: (device: string, event: InputEvent): Promise<void> => (DEMO ? demo.sendInput(device, event) : invoke("send_input", { device, event })),
  appVersion: (): Promise<string> => (DEMO ? Promise.resolve("demo") : invoke("app_version")),
  permissions: (): Promise<Permissions> => (DEMO ? demo.permissions() : invoke("permissions")),
  requestPermission: (kind: PermissionKind): Promise<void> => (DEMO ? demo.requestPermission(kind) : invoke("request_permission", { kind })),
  /** Restarts OmniDesk (macOS only applies Screen Recording after a restart). */
  relaunch: async (): Promise<void> => {
    if (DEMO) return window.location.reload();
    const { relaunch } = await import("@tauri-apps/plugin-process");
    await relaunch();
  },
};

export function onDevices(cb: (devices: Device[]) => void): () => void {
  return DEMO ? demo.onDevices(cb) : listen<Device[]>("node://devices", cb);
}

export function onEvent<T = string>(name: "stream-error" | "paired" | "pairing-ended" | "notice", cb: (payload: T) => void): () => void {
  return DEMO ? demo.onEvent(name, cb as (p: unknown) => void) : listen<T>(`node://${name}`, cb);
}

// ---------------------------------------------------------------- streams

/**
 * Several views can show the same screen (the wall tile and the full-size view while switching
 * between them). Each holds a claim; the device is asked for one stream at the best quality any
 * claim wants, and told to stop only when the last claim is gone. Requests go out one at a time,
 * in order, so a stop can never overtake the start that replaced it.
 */
const claims = new Map<string, StreamMode[]>();
const sent = new Map<string, StreamMode>();
let queue: Promise<unknown> = Promise.resolve();
let flushScheduled = false;

function flushStreams() {
  flushScheduled = false;
  const keys = new Set([...claims.keys(), ...sent.keys()]);
  for (const key of keys) {
    const modes = claims.get(key) ?? [];
    const want: StreamMode | null = modes.length === 0 ? null : modes.includes("focus") ? "focus" : "thumbnail";
    if (want === (sent.get(key) ?? null)) continue;
    const cut = key.lastIndexOf("-");
    const device = key.slice(0, cut);
    const screen = Number(key.slice(cut + 1));
    if (want) sent.set(key, want);
    else sent.delete(key);
    queue = queue.then(() => (want ? api.watch(device, screen, want) : api.unwatch(device, screen))).catch(() => {});
  }
}

/** Starts (or upgrades) the stream of one screen. Call the returned function to let go of it. */
export function claimStream(device: string, screen: number, mode: StreamMode): () => void {
  const key = `${device}-${screen}`;
  claims.set(key, [...(claims.get(key) ?? []), mode]);
  const schedule = () => {
    if (!flushScheduled) {
      flushScheduled = true;
      // Unmounting one view and mounting the next happen in the same tick: decide once, after both.
      setTimeout(flushStreams, 0);
    }
  };
  schedule();
  let released = false;
  return () => {
    if (released) return;
    released = true;
    const list = claims.get(key) ?? [];
    list.splice(list.indexOf(mode), 1);
    if (list.length === 0) claims.delete(key);
    schedule();
  };
}

/** After a device reconnects, its streams are requested again by the node itself. */
export function forgetStreams(device: string) {
  for (const key of [...sent.keys()]) if (key.startsWith(`${device}-`)) sent.delete(key);
  flushStreams();
}

// ---------------------------------------------------------------- frames

type FrameListener = (frame: FrameEvent) => void;
const frameListeners = new Map<string, Set<FrameListener>>();
let frameSource: (() => void) | null = null;

function dispatchFrame(frame: FrameEvent) {
  frameListeners.get(`${frame.device}-${frame.screen}`)?.forEach((cb) => cb(frame));
}

/** Calls `cb` for every new frame of one screen. Start the stream itself with `api.watch`. */
export function onFrame(device: string, screen: number, cb: FrameListener): () => void {
  const key = `${device}-${screen}`;
  if (!frameListeners.has(key)) frameListeners.set(key, new Set());
  frameListeners.get(key)!.add(cb);
  if (!frameSource) {
    if (DEMO) {
      frameSource = demo.onFrame(dispatchFrame);
    } else {
      const urlFor = import("@tauri-apps/api/core").then((core) => core.convertFileSrc);
      frameSource = listen<Omit<FrameEvent, "url">>("node://frame", (f) => {
        void urlFor.then((convert) => dispatchFrame({ ...f, url: `${convert(`${f.device}-${f.screen}`, "frame")}?s=${f.seq}` }));
      });
    }
  }
  return () => {
    frameListeners.get(key)?.delete(cb);
  };
}

// ---------------------------------------------------------------- start at sign-in

export const autostart = {
  async get(): Promise<boolean> {
    if (DEMO) return demo.state.autostart;
    const { isEnabled } = await import("@tauri-apps/plugin-autostart");
    return isEnabled();
  },
  async set(on: boolean): Promise<void> {
    if (DEMO) {
      demo.state.autostart = on;
      return;
    }
    const { enable, disable } = await import("@tauri-apps/plugin-autostart");
    await (on ? enable() : disable());
  },
};

// ---------------------------------------------------------------- auto-update

export interface PendingUpdate {
  version: string;
  notes?: string;
  install: (onProgress: (pct: number) => void) => Promise<void>;
}

/** Checks GitHub Releases for a newer signed build. Returns null when up to date. */
export async function checkForUpdate(): Promise<PendingUpdate | null> {
  if (DEMO) return null;
  const { check } = await import("@tauri-apps/plugin-updater");
  const { relaunch } = await import("@tauri-apps/plugin-process");
  const update = await check();
  if (!update) return null;
  return {
    version: update.version,
    notes: update.body ?? undefined,
    install: async (onProgress) => {
      let total = 0;
      let done = 0;
      await update.downloadAndInstall((ev) => {
        if (ev.event === "Started") total = ev.data.contentLength ?? 0;
        if (ev.event === "Progress") {
          done += ev.data.chunkLength;
          if (total) onProgress(Math.round((done / total) * 100));
        }
        if (ev.event === "Finished") onProgress(100);
      });
      await relaunch();
    },
  };
}

export type Os = "windows" | "macos" | "linux" | "ios" | "android";
export type Kind = "desktop" | "laptop" | "phone" | "tablet";

export interface Capabilities {
  view: "none" | "on_demand" | "always";
  control: boolean;
  grab: boolean;
  display_sink: boolean;
  display_source: boolean;
  clipboard: boolean;
}

export interface ScreenInfo {
  index: number;
  name: string;
  x: number;
  y: number;
  width: number;
  height: number;
  scale: number;
  primary: boolean;
}

export interface Device {
  id: string;
  name: string;
  kind: Kind;
  os: Os;
  paired: boolean;
  online: boolean;
  nearby: boolean;
  address: string | null;
  capabilities: Capabilities | null;
  screens: ScreenInfo[];
  app_version: string | null;
}

export interface Settings {
  name: string | null;
  allow_view: boolean;
  allow_control: boolean;
  share_clipboard: boolean;
}

export interface Self {
  info: { id: string; name: string; kind: Kind; os: Os; screens: ScreenInfo[]; capabilities: Capabilities };
  settings: Settings;
  address: string | null;
  port: number;
}

export interface PairingOffer {
  code: string;
  seconds: number;
}

export interface FrameEvent {
  device: string;
  screen: number;
  seq: number;
  width: number;
  height: number;
  /** Where the picture for this frame can be loaded from. */
  url: string;
}

export type MouseButton = "left" | "right" | "middle" | "back" | "forward";

/** Same shape as `InputEvent` in crates/omnidesk-proto. */
export type InputEvent =
  | { pointer_abs: { screen: number; x: number; y: number } }
  | { button: { button: MouseButton; down: boolean } }
  | { scroll: { dx: number; dy: number } }
  | { key: { code: string; key: string; down: boolean } }
  | { text: { text: string } };

export type StreamMode = "thumbnail" | "focus";

export type Access = "granted" | "missing" | "not_needed";
export type PermissionKind = "screen" | "control";
export interface Permissions {
  screen: Access;
  control: Access;
}

//! Flow: one keyboard and mouse across every screen.
//!
//! Screens of all devices are placed in one shared arrangement, like monitors in the display
//! settings of a single computer. [`Flow`] follows the pointer through that arrangement and says
//! when it leaves this device (grab the local keyboard and mouse and forward them), where it is
//! on the other device, and when it comes back.
//!
//! This crate is pure logic with no OS calls, so it behaves the same on every platform. The
//! platform layer feeds it pointer motion and acts on the [`Step`] it returns.

use omnidesk_proto::{DeviceId, ScreenInfo};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
    fn overlaps(&self, o: &Rect) -> bool {
        self.x < o.x + o.w && o.x < self.x + self.w && self.y < o.y + o.h && o.y < self.y + self.h
    }
}

/// One screen of one device, placed in the shared arrangement.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Placed {
    pub device: DeviceId,
    pub screen: u32,
    pub rect: Rect,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Layout {
    pub screens: Vec<Placed>,
}

impl Layout {
    pub fn screen_at(&self, x: i32, y: i32) -> Option<&Placed> {
        self.screens.iter().find(|s| s.rect.contains(x, y))
    }

    pub fn find(&self, device: &DeviceId, screen: u32) -> Option<&Placed> {
        self.screens
            .iter()
            .find(|s| s.device == *device && s.screen == screen)
    }

    /// True when no two screens overlap and every screen has a size.
    pub fn is_valid(&self) -> bool {
        self.screens.iter().all(|s| s.rect.w > 0 && s.rect.h > 0)
            && self.screens.iter().enumerate().all(|(i, a)| {
                self.screens[i + 1..]
                    .iter()
                    .all(|b| !a.rect.overlaps(&b.rect))
            })
    }

    /// Default arrangement: devices side by side from left to right, top edges aligned, each
    /// device keeping the relative positions of its own monitors.
    pub fn row(devices: &[(DeviceId, Vec<ScreenInfo>)]) -> Layout {
        let mut screens = Vec::new();
        let mut cursor_x = 0i32;
        for (device, list) in devices {
            if list.is_empty() {
                continue;
            }
            let min_x = list.iter().map(|s| s.x).min().unwrap_or(0);
            let min_y = list.iter().map(|s| s.y).min().unwrap_or(0);
            let max_x = list.iter().map(|s| s.x + s.width as i32).max().unwrap_or(0);
            for s in list {
                screens.push(Placed {
                    device: device.clone(),
                    screen: s.index,
                    rect: Rect {
                        x: cursor_x + (s.x - min_x),
                        y: s.y - min_y,
                        w: s.width as i32,
                        h: s.height as i32,
                    },
                });
            }
            cursor_x += max_x - min_x;
        }
        Layout { screens }
    }
}

/// A point on one screen. `x` and `y` run from 0.0 (left / top) to 1.0 (right / bottom).
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub device: DeviceId,
    pub screen: u32,
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// The pointer is on this device and stays there. Nothing to forward.
    Stay,
    /// The pointer moved onto another device. Grab local input (if not already grabbed) and
    /// forward everything to `Target.device`. If it was on a different remote device before,
    /// that one is no longer the target.
    Enter(Target),
    /// The pointer moved within the remote device it was already on.
    Move(Target),
    /// The pointer came back to this device. Release the grab and put the local pointer there.
    Return(Target),
}

pub struct Flow {
    layout: Layout,
    local: DeviceId,
    owner: DeviceId,
    pos: (i32, i32),
}

impl Flow {
    pub fn new(layout: Layout, local: DeviceId) -> Flow {
        let pos = layout
            .screens
            .iter()
            .find(|s| s.device == local)
            .map(|s| (s.rect.x + s.rect.w / 2, s.rect.y + s.rect.h / 2))
            .unwrap_or((0, 0));
        Flow {
            layout,
            owner: local.clone(),
            local,
            pos,
        }
    }

    /// Device the pointer is on right now.
    pub fn owner(&self) -> &DeviceId {
        &self.owner
    }

    pub fn is_remote(&self) -> bool {
        self.owner != self.local
    }

    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    fn target(&self, placed: &Placed) -> Target {
        let r = placed.rect;
        let fx = if r.w > 1 {
            (self.pos.0 - r.x) as f32 / (r.w - 1) as f32
        } else {
            0.0
        };
        let fy = if r.h > 1 {
            (self.pos.1 - r.y) as f32 / (r.h - 1) as f32
        } else {
            0.0
        };
        Target {
            device: placed.device.clone(),
            screen: placed.screen,
            x: fx.clamp(0.0, 1.0),
            y: fy.clamp(0.0, 1.0),
        }
    }

    /// The local pointer moved while this device has it.
    ///
    /// `x`, `y` are the pointer position in pixels on local screen `screen` after the move
    /// (as clamped by the OS); `dx`, `dy` the raw motion that caused it. The pointer crosses to
    /// a neighbour only when it sits on an edge and keeps pushing outwards.
    pub fn local_motion(&mut self, screen: u32, x: i32, y: i32, dx: i32, dy: i32) -> Step {
        if self.is_remote() {
            return self.remote_motion(dx, dy);
        }
        let Some(here) = self.layout.find(&self.local, screen).cloned() else {
            return Step::Stay;
        };
        let r = here.rect;
        let px = r.x + x.clamp(0, r.w - 1);
        let py = r.y + y.clamp(0, r.h - 1);
        self.pos = (px, py);

        let candidates = [
            (dx < 0 && px == r.x, (px - 1, py)),
            (dx > 0 && px == r.x + r.w - 1, (px + 1, py)),
            (dy < 0 && py == r.y, (px, py - 1)),
            (dy > 0 && py == r.y + r.h - 1, (px, py + 1)),
        ];
        for (pushing, (nx, ny)) in candidates {
            if !pushing {
                continue;
            }
            if let Some(next) = self.layout.screen_at(nx, ny).cloned() {
                if next.device != self.local {
                    self.pos = (nx, ny);
                    self.owner = next.device.clone();
                    return Step::Enter(self.target(&next));
                }
            }
        }
        Step::Stay
    }

    /// Moves the pointer by (`dx`, `dy`) through the arrangement: freely where there is a
    /// screen, across shared edges, and sliding along edges that have nothing behind them.
    fn travel(&self, dx: i32, dy: i32) -> (i32, i32) {
        let (mut x, mut y) = self.pos;
        let (mut rx, mut ry) = (dx, dy);
        // Each pass either finishes or crosses one edge; a few passes cover any real motion.
        for _ in 0..8 {
            if self.layout.screen_at(x + rx, y + ry).is_some() {
                return (x + rx, y + ry);
            }
            let Some(cur) = self.layout.screen_at(x, y).map(|s| s.rect) else {
                return (x, y);
            };
            let cx = (x + rx).clamp(cur.x, cur.x + cur.w - 1);
            let cy = (y + ry).clamp(cur.y, cur.y + cur.h - 1);
            rx -= cx - x;
            ry -= cy - y;
            x = cx;
            y = cy;
            // Standing on an edge with motion left over: step across if a screen continues there.
            if rx != 0 && self.layout.screen_at(x + rx.signum(), y).is_some() {
                x += rx.signum();
                rx -= rx.signum();
            } else if ry != 0 && self.layout.screen_at(x, y + ry.signum()).is_some() {
                y += ry.signum();
                ry -= ry.signum();
            } else {
                return (x, y);
            }
        }
        (x, y)
    }

    /// Relative motion while the pointer is on another device (local input is grabbed).
    pub fn remote_motion(&mut self, dx: i32, dy: i32) -> Step {
        if !self.is_remote() {
            return Step::Stay;
        }
        let (nx, ny) = self.travel(dx, dy);
        self.pos = (nx, ny);
        let Some(placed) = self.layout.screen_at(nx, ny).cloned() else {
            return self.force_local();
        };
        let target = self.target(&placed);
        if placed.device == self.local {
            self.owner = self.local.clone();
            Step::Return(target)
        } else if placed.device != self.owner {
            self.owner = placed.device.clone();
            Step::Enter(target)
        } else {
            Step::Move(target)
        }
    }

    /// Brings the pointer back to this device (escape shortcut, target disconnected).
    pub fn force_local(&mut self) -> Step {
        self.owner = self.local.clone();
        match self
            .layout
            .screens
            .iter()
            .find(|s| s.device == self.local)
            .cloned()
        {
            Some(home) => {
                self.pos = (home.rect.x + home.rect.w / 2, home.rect.y + home.rect.h / 2);
                Step::Return(self.target(&home))
            }
            None => Step::Stay,
        }
    }

    /// Replaces the arrangement. If the device the pointer was on is gone, the pointer comes home.
    pub fn set_layout(&mut self, layout: Layout) -> Step {
        self.layout = layout;
        if self.is_remote()
            && self
                .layout
                .screen_at(self.pos.0, self.pos.1)
                .map(|s| &s.device)
                != Some(&self.owner)
        {
            return self.force_local();
        }
        Step::Stay
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(s: &str) -> DeviceId {
        DeviceId(s.to_string())
    }
    fn placed(device: &str, screen: u32, x: i32, y: i32, w: i32, h: i32) -> Placed {
        Placed {
            device: id(device),
            screen,
            rect: Rect { x, y, w, h },
        }
    }
    fn info(index: u32, x: i32, y: i32, w: u32, h: u32) -> ScreenInfo {
        ScreenInfo {
            index,
            name: format!("Display {index}"),
            x,
            y,
            width: w,
            height: h,
            scale: 1.0,
            primary: index == 0,
        }
    }

    /// PC (1920x1080) with the MacBook (1440x900) to its right.
    fn pc_and_mac() -> Flow {
        Flow::new(
            Layout {
                screens: vec![
                    placed("pc", 0, 0, 0, 1920, 1080),
                    placed("mac", 0, 1920, 0, 1440, 900),
                ],
            },
            id("pc"),
        )
    }

    #[test]
    fn moving_inside_the_screen_stays_local() {
        let mut f = pc_and_mac();
        assert_eq!(f.local_motion(0, 500, 500, 30, -4), Step::Stay);
        assert!(!f.is_remote());
    }

    #[test]
    fn pushing_through_the_shared_edge_enters_the_neighbour() {
        let mut f = pc_and_mac();
        match f.local_motion(0, 1919, 450, 6, 0) {
            Step::Enter(t) => {
                assert_eq!(t.device, id("mac"));
                assert_eq!(t.x, 0.0);
                assert!((t.y - 450.0 / 899.0).abs() < 1e-4);
            }
            other => panic!("expected Enter, got {other:?}"),
        }
        assert_eq!(f.owner(), &id("mac"));
    }

    #[test]
    fn resting_on_the_edge_without_pushing_does_not_cross() {
        let mut f = pc_and_mac();
        assert_eq!(f.local_motion(0, 1919, 450, 0, 3), Step::Stay);
        assert_eq!(f.local_motion(0, 1919, 450, -2, 0), Step::Stay);
    }

    #[test]
    fn edges_without_a_neighbour_do_nothing() {
        let mut f = pc_and_mac();
        assert_eq!(f.local_motion(0, 0, 300, -9, 0), Step::Stay);
        assert_eq!(f.local_motion(0, 700, 0, 0, -9), Step::Stay);
        // Bottom 180 px of the PC's right edge has no MacBook next to it (900 < 1080).
        assert_eq!(f.local_motion(0, 1919, 1000, 9, 0), Step::Stay);
    }

    #[test]
    fn pointer_moves_on_the_remote_screen_and_comes_back() {
        let mut f = pc_and_mac();
        f.local_motion(0, 1919, 100, 5, 0);
        match f.remote_motion(100, 50) {
            Step::Move(t) => {
                assert_eq!(t.device, id("mac"));
                assert!((t.x - 100.0 / 1439.0).abs() < 1e-4);
                assert!((t.y - 150.0 / 899.0).abs() < 1e-4);
            }
            other => panic!("expected Move, got {other:?}"),
        }
        match f.remote_motion(-150, 0) {
            Step::Return(t) => {
                assert_eq!(t.device, id("pc"));
                assert!(t.x > 0.97 && t.x <= 1.0);
            }
            other => panic!("expected Return, got {other:?}"),
        }
        assert!(!f.is_remote());
        assert_eq!(f.remote_motion(5, 5), Step::Stay);
    }

    #[test]
    fn remote_pointer_slides_along_outer_edges() {
        let mut f = pc_and_mac();
        f.local_motion(0, 1919, 100, 5, 0);
        // Far right and up: stops in the top-right corner of the MacBook.
        assert_eq!(
            f.remote_motion(5000, -5000),
            Step::Move(Target {
                device: id("mac"),
                screen: 0,
                x: 1.0,
                y: 0.0
            })
        );
        // Down past the bottom while moving left a little: x moves, y is clamped.
        match f.remote_motion(-10, 5000) {
            Step::Move(t) => {
                assert_eq!(t.y, 1.0);
                assert!((t.x - 1429.0 / 1439.0).abs() < 1e-4);
            }
            other => panic!("expected Move, got {other:?}"),
        }
    }

    #[test]
    fn three_devices_in_a_row_hand_over_between_remotes() {
        let layout = Layout {
            screens: vec![
                placed("pc", 0, 0, 0, 1000, 1000),
                placed("mac", 0, 1000, 0, 1000, 1000),
                placed("tab", 0, 2000, 0, 1000, 1000),
            ],
        };
        let mut f = Flow::new(layout, id("pc"));
        assert!(
            matches!(f.local_motion(0, 999, 500, 1, 0), Step::Enter(t) if t.device == id("mac"))
        );
        assert!(matches!(f.remote_motion(500, 0), Step::Move(_)));
        assert!(matches!(f.remote_motion(600, 0), Step::Enter(t) if t.device == id("tab")));
        assert_eq!(f.owner(), &id("tab"));
        assert!(matches!(f.remote_motion(-700, 0), Step::Enter(t) if t.device == id("mac")));
        assert!(matches!(f.remote_motion(-1200, 0), Step::Return(t) if t.device == id("pc")));
    }

    #[test]
    fn own_second_monitor_is_left_to_the_os() {
        let layout = Layout {
            screens: vec![
                placed("pc", 0, 0, 0, 1000, 1000),
                placed("pc", 1, 1000, 0, 1000, 1000),
                placed("mac", 0, 2000, 0, 1000, 1000),
            ],
        };
        let mut f = Flow::new(layout, id("pc"));
        assert_eq!(f.local_motion(0, 999, 500, 4, 0), Step::Stay);
        assert!(
            matches!(f.local_motion(1, 999, 500, 4, 0), Step::Enter(t) if t.device == id("mac"))
        );
    }

    #[test]
    fn neighbour_above_and_below() {
        let layout = Layout {
            screens: vec![
                placed("pc", 0, 0, 1000, 1000, 1000),
                placed("tab", 0, 0, 0, 1000, 1000),
            ],
        };
        let mut f = Flow::new(layout, id("pc"));
        match f.local_motion(0, 250, 0, 0, -3) {
            Step::Enter(t) => {
                assert_eq!(t.device, id("tab"));
                assert_eq!(t.y, 1.0);
            }
            other => panic!("expected Enter, got {other:?}"),
        }
        assert!(
            matches!(f.remote_motion(0, 5), Step::Return(t) if t.device == id("pc") && t.y < 0.01)
        );
    }

    #[test]
    fn escape_and_layout_change_bring_the_pointer_home() {
        let mut f = pc_and_mac();
        f.local_motion(0, 1919, 100, 5, 0);
        assert!(
            matches!(f.force_local(), Step::Return(t) if t.device == id("pc") && (t.x - 0.5).abs() < 0.01)
        );
        f.local_motion(0, 1919, 100, 5, 0);
        assert!(f.is_remote());
        // The MacBook disconnects and is removed from the arrangement.
        let step = f.set_layout(Layout {
            screens: vec![placed("pc", 0, 0, 0, 1920, 1080)],
        });
        assert!(matches!(step, Step::Return(_)));
        assert!(!f.is_remote());
    }

    #[test]
    fn default_row_layout_is_adjacent_and_valid() {
        let layout = Layout::row(&[
            (
                id("pc"),
                vec![info(0, 0, 0, 2560, 1440), info(1, 2560, 180, 1920, 1080)],
            ),
            (id("mac"), vec![info(0, 0, 0, 1512, 982)]),
            (id("headless"), vec![]),
        ]);
        assert!(layout.is_valid());
        assert_eq!(
            layout.find(&id("pc"), 1).unwrap().rect,
            Rect {
                x: 2560,
                y: 180,
                w: 1920,
                h: 1080
            }
        );
        assert_eq!(
            layout.find(&id("mac"), 0).unwrap().rect,
            Rect {
                x: 4480,
                y: 0,
                w: 1512,
                h: 982
            }
        );
        // Crossing from the PC's second monitor reaches the MacBook.
        let mut f = Flow::new(layout, id("pc"));
        assert!(
            matches!(f.local_motion(1, 1919, 300, 2, 0), Step::Enter(t) if t.device == id("mac"))
        );
    }

    #[test]
    fn a_fast_flick_crosses_instead_of_sticking_to_the_edge() {
        let mut f = pc_and_mac();
        f.local_motion(0, 1919, 100, 5, 0);
        f.remote_motion(700, 0);
        // One huge motion to the left overshoots the PC's far edge: it must land on the PC.
        match f.remote_motion(-9000, 0) {
            Step::Return(t) => {
                assert_eq!(t.device, id("pc"));
                assert_eq!(t.x, 0.0);
            }
            other => panic!("expected Return, got {other:?}"),
        }
    }

    #[test]
    fn overlapping_layouts_are_invalid() {
        let layout = Layout {
            screens: vec![
                placed("pc", 0, 0, 0, 1000, 1000),
                placed("mac", 0, 500, 500, 1000, 1000),
            ],
        };
        assert!(!layout.is_valid());
    }
}

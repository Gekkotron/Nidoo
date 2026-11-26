//! Pan, zoom, fit and minimap maths for the flow canvas. Pure functions, no DOM.

use crate::flow_layout::{NODE_H, NODE_W, PAD, Pt, Rect};

/// Lowest zoom reachable by wheel, pinch or the buttons.
pub const ZOOM_MIN: f64 = 0.25;
/// Highest zoom reachable by wheel, pinch or the buttons.
pub const ZOOM_MAX: f64 = 2.0;
/// Lowest zoom `fit` picks.
pub const FIT_MIN: f64 = 0.3;
/// Highest zoom `fit` picks, so a single card is not blown up.
pub const FIT_MAX: f64 = 1.0;
/// Multiplicative step of the +/− buttons.
pub const ZOOM_STEP: f64 = 1.2;
/// Screen width the side panel covers on the right of the canvas: `--drawer-w` (360px) plus
/// its 24px of margin in `main.css`; keep the two in step.
pub const PANEL_RESERVE_PX: f64 = 384.0;

/// Pan and zoom of the canvas: `screen = world * zoom + (x, y)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub x: f64,
    pub y: f64,
    pub zoom: f64,
}

impl View {
    /// Screen point to world point.
    pub fn to_world(&self, screen: Pt) -> Pt {
        Pt {
            x: (screen.x - self.x) / self.zoom,
            y: (screen.y - self.y) / self.zoom,
        }
    }

    /// World point to screen point.
    pub fn to_screen(&self, world: Pt) -> Pt {
        Pt {
            x: world.x * self.zoom + self.x,
            y: world.y * self.zoom + self.y,
        }
    }

    /// Scales by `factor` keeping the world point under `screen` fixed; the
    /// zoom is clamped to `[ZOOM_MIN, ZOOM_MAX]`.
    pub fn zoom_at(&self, screen: Pt, factor: f64) -> View {
        let zoom = (self.zoom * factor).clamp(ZOOM_MIN, ZOOM_MAX);
        let w = self.to_world(screen);
        View {
            x: screen.x - w.x * zoom,
            y: screen.y - w.y * zoom,
            zoom,
        }
    }

    /// Moves the view by a screen-space delta.
    pub fn pan(&self, dx: f64, dy: f64) -> View {
        View {
            x: self.x + dx,
            y: self.y + dy,
            zoom: self.zoom,
        }
    }

    /// The CSS transform of the world layer.
    pub fn css_transform(&self) -> String {
        format!("translate({}px,{}px) scale({})", self.x, self.y, self.zoom)
    }
}

/// A view framing `content` (inflated by `PAD`), centred, with the zoom in
/// `[FIT_MIN, FIT_MAX]`.
pub fn fit(content: Rect, screen_w: f64, screen_h: f64) -> View {
    fit_beside(content, screen_w, screen_h, 0.0)
}

/// [`fit`] in the part of the screen a side panel `right_px` wide leaves free on the right,
/// or in the whole screen when that part is narrower than a card.
pub fn fit_beside(content: Rect, screen_w: f64, screen_h: f64, right_px: f64) -> View {
    let free_w = free_width(screen_w, right_px, NODE_W);
    let w = (content.w + 2.0 * PAD).max(1.0);
    let h = (content.h + 2.0 * PAD).max(1.0);
    let zoom = (free_w / w).min(screen_h / h).clamp(FIT_MIN, FIT_MAX);
    let cx = content.x + content.w / 2.0;
    let cy = content.y + content.h / 2.0;
    View {
        x: free_w / 2.0 - cx * zoom,
        y: screen_h / 2.0 - cy * zoom,
        zoom,
    }
}

/// The screen width left of a `panel_px` panel, or all of it when what is left is narrower
/// than `at_least`.
fn free_width(screen_w: f64, panel_px: f64, at_least: f64) -> f64 {
    if screen_w - panel_px >= at_least {
        screen_w - panel_px
    } else {
        screen_w
    }
}

/// The world rectangle on screen left of a `panel_px` side panel (all of the screen when what
/// is left is narrower than a card).
pub fn free_area(view: &View, screen_w: f64, screen_h: f64, panel_px: f64) -> Rect {
    let free_w = free_width(screen_w, panel_px, NODE_W * view.zoom);
    let tl = view.to_world(Pt { x: 0.0, y: 0.0 });
    Rect {
        x: tl.x,
        y: tl.y,
        w: free_w / view.zoom,
        h: screen_h / view.zoom,
    }
}

/// The world rectangle currently on screen.
pub fn visible_world(view: &View, screen_w: f64, screen_h: f64) -> Rect {
    let tl = view.to_world(Pt { x: 0.0, y: 0.0 });
    Rect {
        x: tl.x,
        y: tl.y,
        w: screen_w / view.zoom,
        h: screen_h / view.zoom,
    }
}

/// Where a card added from the library lands: centred in the part of the screen the side panel
/// leaves free (`panel_px` on the right), or in the whole screen when that part is narrower
/// than a card.
pub fn add_spot(view: &View, screen_w: f64, screen_h: f64, panel_px: f64) -> Pt {
    let (card_w, card_h) = (NODE_W * view.zoom, NODE_H * view.zoom);
    let free_w = free_width(screen_w, panel_px, card_w);
    view.to_world(Pt {
        x: (free_w - card_w) / 2.0,
        y: (screen_h - card_h) / 2.0,
    })
}

/// World to minimap transform: `m = (world - o) * scale`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mini {
    pub scale: f64,
    pub ox: f64,
    pub oy: f64,
}

/// A minimap transform fitting the union of `content` and `view_rect` into a
/// `w` × `h` box, centred.
pub fn minimap(content: Rect, view_rect: Rect, w: f64, h: f64) -> Mini {
    let x0 = content.x.min(view_rect.x);
    let y0 = content.y.min(view_rect.y);
    let x1 = (content.x + content.w).max(view_rect.x + view_rect.w);
    let y1 = (content.y + content.h).max(view_rect.y + view_rect.h);
    let uw = (x1 - x0).max(1.0);
    let uh = (y1 - y0).max(1.0);
    let scale = (w / uw).min(h / uh);
    Mini {
        scale,
        ox: x0 - (w / scale - uw) / 2.0,
        oy: y0 - (h / scale - uh) / 2.0,
    }
}

/// Minimap point back to world.
pub fn mini_to_world(m: &Mini, p: Pt) -> Pt {
    Pt {
        x: p.x / m.scale + m.ox,
        y: p.y / m.scale + m.oy,
    }
}

/// The view moved so `world` sits in the middle of the screen, zoom kept.
pub fn centre_on(view: &View, world: Pt, screen_w: f64, screen_h: f64) -> View {
    View {
        x: screen_w / 2.0 - world.x * view.zoom,
        y: screen_h / 2.0 - world.y * view.zoom,
        zoom: view.zoom,
    }
}

#[cfg(test)]
mod tests;

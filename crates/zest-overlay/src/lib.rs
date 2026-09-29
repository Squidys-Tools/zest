//! Radial overlay: custom Direct2D vector UI on a transparent layered window.
//!
//! Feel (PRD §How It Looks and Feels): physical dial, dark semi-transparent +
//! acrylic/mica blur, active sector = smoothed 1–3 color gradient, inactive =
//! muted dark gray + thin borders, ~200ms ease-out sector transitions,
//! center = file thumbnail or multi-file count badge, Segoe UI Variable,
//! Lucide icons as starting point.
//!
//! Performance: window is pre-created hidden; activation only reveals it.
//! WinPie (OSS Rust radial menu) is the architectural reference.

use std::f64::consts::{PI, TAU};
use zest_core::{Gradient, HexColor, MenuAction, MenuNode, DEFAULT_GRADIENT};

/// Ease-out duration for sector hover transitions.
pub const SECTOR_TRANSITION_MS: u64 = 200;
/// Frame interval the hover fade runs at, ~60fps.
pub const ANIMATION_FRAME_MS: u64 = 16;
pub const OVERLAY_SIZE: f32 = 256.0;
pub const RING_INNER_RADIUS: f32 = 52.0;
pub const RING_OUTER_RADIUS: f32 = 112.0;

/// Straight-alpha sRGB color, channels in 0.0..=1.0.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgba {
    pub fn lerp(self, other: Self, amount: f32) -> Self {
        let mix = |from: f32, to: f32| from + (to - from) * amount;
        Self {
            r: mix(self.r, other.r),
            g: mix(self.g, other.g),
            b: mix(self.b, other.b),
            a: mix(self.a, other.a),
        }
    }
}

impl From<HexColor> for Rgba {
    fn from(color: HexColor) -> Self {
        let (r, g, b) = color.to_unit_rgb();
        Self { r, g, b, a: 1.0 }
    }
}

/// Parse a `#rrggbb` string — the only form `Gradient` stores.
pub fn parse_hex_color(hex: &str) -> Option<Rgba> {
    let digits = hex.strip_prefix('#')?;
    if digits.len() != 6 || !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let channel = |index: usize| {
        u8::from_str_radix(digits.get(index..index + 2)?, 16)
            .ok()
            .map(|value| value as f32 / 255.0)
    };
    Some(Rgba {
        r: channel(0)?,
        g: channel(2)?,
        b: channel(4)?,
        a: 1.0,
    })
}

/// The gradient the settings window ships with, used when the stored list is
/// empty.
pub fn default_gradient() -> Vec<Rgba> {
    DEFAULT_GRADIENT.iter().copied().map(Rgba::from).collect()
}

/// The settings gradient as brush stops. A single color is doubled because a
/// gradient brush needs at least two stops.
pub fn gradient_stops(colors: &[HexColor]) -> Vec<Rgba> {
    let parsed: Vec<Rgba> = colors.iter().copied().map(Rgba::from).collect();
    match parsed.len() {
        0 => default_gradient(),
        1 => vec![parsed[0], parsed[0]],
        _ => parsed,
    }
}

/// Cubic ease-out: quick start, soft landing. `amount` is clamped to 0.0..=1.0.
pub fn ease_out(amount: f32) -> f32 {
    let amount = amount.clamp(0.0, 1.0);
    1.0 - (1.0 - amount).powi(3)
}

/// One sector's fade: the value it held when the fade began, where it is headed,
/// and how far into `SECTOR_TRANSITION_MS` it is.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Fade {
    start: f32,
    target: f32,
    elapsed: f32,
}

impl Fade {
    fn settled(value: f32) -> Self {
        Self {
            start: value,
            target: value,
            elapsed: SECTOR_TRANSITION_MS as f32,
        }
    }

    fn value(&self) -> f32 {
        self.start
            + (self.target - self.start) * ease_out(self.elapsed / SECTOR_TRANSITION_MS as f32)
    }
}

/// How lit each sector on the current ring is: 0 inactive, 1 fully lit. The
/// renderer advances this a frame at a time so a sector eases over
/// `SECTOR_TRANSITION_MS` rather than snapping, and re-aiming mid-fade carries
/// on from the value already on screen instead of jumping.
#[derive(Debug, Clone, PartialEq)]
pub struct SectorEmphasis {
    active: Option<usize>,
    fades: Vec<Fade>,
}

impl SectorEmphasis {
    /// A ring of `count` sectors with nothing lit.
    pub fn new(count: usize) -> Self {
        Self {
            active: None,
            fades: vec![Fade::settled(0.0); count],
        }
    }

    /// The sector the cursor is over, if any.
    pub fn active(&self) -> Option<usize> {
        self.active
    }

    /// Eased lit amount for `index`. Sectors off the ring read as inactive.
    pub fn value(&self, index: usize) -> f32 {
        self.fades.get(index).map(Fade::value).unwrap_or(0.0)
    }

    /// Eased lit amount for every sector on the ring.
    #[cfg(test)]
    pub fn values(&self) -> Vec<f32> {
        self.fades.iter().map(Fade::value).collect()
    }

    /// Light `active` on a ring of `count` sectors.
    pub fn set_active(&mut self, count: usize, active: Option<usize>) {
        if count != self.fades.len() {
            self.reset(count);
        }
        if self.active == active {
            return;
        }
        for (index, fade) in self.fades.iter_mut().enumerate() {
            let target = Self::target(active, index);
            if target != fade.target {
                *fade = Fade {
                    start: fade.value(),
                    target,
                    elapsed: 0.0,
                };
            }
        }
        self.active = active;
    }

    /// Light nothing — the state a freshly drawn ring starts from.
    pub fn reset(&mut self, count: usize) {
        self.active = None;
        self.fades = vec![Fade::settled(0.0); count];
    }

    /// Run every fade forward by `elapsed_ms`; reports whether any is still moving.
    pub fn advance(&mut self, elapsed_ms: u64) -> bool {
        let mut running = false;
        for fade in self.fades.iter_mut() {
            if (fade.start - fade.target).abs() <= f32::EPSILON {
                *fade = Fade::settled(fade.target);
                continue;
            }
            fade.elapsed = (fade.elapsed + elapsed_ms as f32).min(SECTOR_TRANSITION_MS as f32);
            if fade.elapsed >= SECTOR_TRANSITION_MS as f32 {
                *fade = Fade::settled(fade.target);
            } else {
                running = true;
            }
        }
        running
    }

    fn target(active: Option<usize>, index: usize) -> f32 {
        if active == Some(index) {
            1.0
        } else {
            0.0
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Sector {
    pub label: String,
    /// Center angle in radians, starting at -π/2.
    pub center: f64,
    /// Angular width in radians.
    pub width: f64,
}

/// Evenly fan `labels` around a full circle starting at top (-90°).
pub fn layout_sectors(labels: &[String]) -> Vec<Sector> {
    let n = labels.len() as f64;
    if n == 0.0 {
        return vec![];
    }
    labels
        .iter()
        .enumerate()
        .map(|(i, label)| Sector {
            label: label.clone(),
            center: -PI / 2.0 + (i as f64 / n) * TAU,
            width: TAU / n,
        })
        .collect()
}

/// Hit-test a cursor angle (radians) against sectors.
pub fn hit_test(sectors: &[Sector], angle: f64) -> Option<usize> {
    if sectors.is_empty() {
        return None;
    }
    let norm = angle.rem_euclid(TAU);
    sectors.iter().position(|s| {
        let half = s.width / 2.0;
        let diff = (norm - s.center).rem_euclid(TAU);
        let dist = diff.min(TAU - diff);
        dist <= half
    })
}

/// Hit-test a point in the overlay's local coordinates.
pub fn hit_test_at_point(sectors: &[Sector], point: (f32, f32)) -> Option<usize> {
    let center = OVERLAY_SIZE / 2.0;
    let dx = point.0 - center;
    let dy = point.1 - center;
    let radius = (dx * dx + dy * dy).sqrt();
    if !(RING_INNER_RADIUS..=RING_OUTER_RADIUS).contains(&radius) {
        return None;
    }
    hit_test(sectors, (dy as f64).atan2(dx as f64))
}

/// What a click on a sector did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Click {
    /// Nothing to do (dead centre, or a category that cannot fan out).
    Miss,
    /// Ring 1 replaced by that category's ring 2.
    Expanded,
    /// A leaf was picked: this is the action the app has to run.
    Action(MenuAction),
}

/// The menu the overlay currently shows, plus where in it we are.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct RingModel {
    /// Whole menu, cloned from the app. Only labels and children matter here.
    menu: Vec<MenuNode>,
    /// Indices picked on the way down; empty means ring 1.
    path: Vec<usize>,
    /// Geometry of the ring currently on screen.
    sectors: Vec<Sector>,
}

impl RingModel {
    pub(crate) fn new(menu: Vec<MenuNode>) -> Self {
        let mut model = Self {
            menu,
            path: Vec::new(),
            sectors: Vec::new(),
        };
        model.relayout();
        model
    }

    /// The nodes drawn on the current ring.
    fn current(&self) -> &[MenuNode] {
        match self.path.split_first() {
            None => &self.menu,
            Some((index, _)) => &self.menu[*index].children,
        }
    }

    fn relayout(&mut self) {
        let labels: Vec<String> = self
            .current()
            .iter()
            .map(|node| node.label.clone())
            .collect();
        self.sectors = layout_sectors(&labels);
    }

    pub(crate) fn hit_test(&self, point: (f32, f32)) -> Option<usize> {
        hit_test_at_point(&self.sectors, point)
    }

    /// Resolve a click: a category fans out, a leaf hands back its action.
    pub(crate) fn click(&mut self, index: usize) -> Click {
        match self.current().get(index) {
            None => Click::Miss,
            Some(node) if node.children.is_empty() => match node.action.clone() {
                Some(action) => Click::Action(action),
                None => Click::Miss,
            },
            Some(_) => {
                self.path.push(index);
                self.relayout();
                Click::Expanded
            }
        }
    }
}

/// What the app hands over for the next showing: the ring to draw, plus the
/// active-sector gradient the user configured.
pub(crate) struct PendingOverlay {
    pub(crate) ring: RingModel,
    pub(crate) gradient: Gradient,
}

/// Menu choices picked by the user, streamed from the overlay thread.
pub struct MenuChoices {
    receiver: tokio::sync::mpsc::UnboundedReceiver<MenuAction>,
}

impl MenuChoices {
    /// The action behind the leaf the user picked, or `None` once the overlay
    /// thread is gone.
    pub async fn recv(&mut self) -> Option<MenuAction> {
        self.receiver.recv().await
    }
}

#[cfg(windows)]
mod win;

#[cfg(windows)]
pub use win::Overlay;

#[cfg(test)]
mod tests {
    use super::*;

    fn menu(labels: &[&str], children: &[&[&str]]) -> Vec<MenuNode> {
        labels
            .iter()
            .zip(children)
            .map(|(label, child_labels)| MenuNode {
                label: (*label).to_string(),
                action: None,
                children: child_labels
                    .iter()
                    .map(|child| MenuNode {
                        label: (*child).to_string(),
                        action: Some(MenuAction::Convert {
                            ext: (*child).to_string(),
                        }),
                        children: Vec::new(),
                    })
                    .collect(),
            })
            .collect()
    }

    #[test]
    fn three_sectors_cover_circle() {
        let labels = vec!["A".into(), "B".into(), "C".into()];
        let sectors = layout_sectors(&labels);
        assert_eq!(sectors.len(), 3);
        assert!(hit_test(&sectors, 0.0).is_some());
    }

    #[test]
    fn first_sector_starts_at_top() {
        let sectors = layout_sectors(&["A".into()]);
        assert_eq!(sectors[0].center, -PI / 2.0);
        assert_eq!(hit_test(&sectors, -PI / 2.0), Some(0));
    }

    #[test]
    fn point_hit_test_respects_center_dead_zone() {
        let sectors = layout_sectors(&["A".into(), "B".into()]);
        assert_eq!(hit_test_at_point(&sectors, (128.0, 128.0)), None);
        assert_eq!(hit_test_at_point(&sectors, (128.0, 28.0)), Some(0));
    }

    #[test]
    fn click_expands_then_yields_the_leaf_action() {
        let mut ring = RingModel::new(menu(&["Convert", "Archive"], &[&["png"], &[]]));
        assert_eq!(ring.click(0), Click::Expanded);
        assert_eq!(ring.sectors[0].label, "png");
        assert_eq!(
            ring.click(0),
            Click::Action(MenuAction::Convert {
                ext: "png".to_string()
            })
        );
    }

    #[test]
    fn clicking_an_empty_category_is_a_miss() {
        let mut ring = RingModel::new(menu(&["Convert", "Archive"], &[&["png"], &[]]));
        assert_eq!(ring.click(1), Click::Miss);
        assert_eq!(ring.sectors.len(), 2);
    }

    #[test]
    fn out_of_range_click_is_a_miss() {
        let mut ring = RingModel::new(Vec::new());
        assert_eq!(ring.click(0), Click::Miss);
    }

    #[test]
    fn empty_has_no_hit() {
        assert_eq!(hit_test(&[], 1.0), None);
    }

    #[test]
    fn parses_settings_hex_colors() {
        assert_eq!(
            parse_hex_color("#ff8a3d"),
            Some(Rgba {
                r: 1.0,
                g: 0x8a as f32 / 255.0,
                b: 0x3d as f32 / 255.0,
                a: 1.0
            })
        );
        assert_eq!(
            parse_hex_color("#000000").map(|c| (c.r, c.g, c.b)),
            Some((0.0, 0.0, 0.0))
        );
    }

    #[test]
    fn rejects_colors_settings_cannot_hold() {
        for bad in ["ff8a3d", "#fff", "#ff8a3dz", "", "#ff8a3"] {
            assert_eq!(parse_hex_color(bad), None, "{bad} should not parse");
        }
    }

    #[test]
    fn gradient_stops_follow_the_settings_list() {
        let stops = gradient_stops(&[
            HexColor::new(0x00, 0x00, 0x00),
            HexColor::new(0xff, 0xff, 0xff),
        ]);
        assert_eq!(stops.len(), 2);
        assert_eq!(stops[0].r, 0.0);
        assert_eq!(stops[1].r, 1.0);
    }

    #[test]
    fn a_single_gradient_color_becomes_two_stops() {
        let stops = gradient_stops(&[HexColor::new(0x12, 0x34, 0x56)]);
        assert_eq!(stops.len(), 2);
        assert_eq!(stops[0], stops[1]);
    }

    #[test]
    fn an_empty_gradient_falls_back_to_the_default() {
        // Stops are typed now, so an unparseable color is not representable;
        // an empty list is the only way to have nothing to draw.
        assert_eq!(gradient_stops(&[]), default_gradient());
    }

    #[test]
    fn ease_out_is_pinned_at_both_ends() {
        assert_eq!(ease_out(0.0), 0.0);
        assert_eq!(ease_out(1.0), 1.0);
        assert_eq!(ease_out(-1.0), 0.0);
        assert_eq!(ease_out(2.0), 1.0);
        assert!(
            ease_out(0.5) > 0.5,
            "ease-out must lead its linear equivalent"
        );
    }

    #[test]
    fn a_sector_lights_over_the_transition_and_settles() {
        let mut emphasis = SectorEmphasis::new(3);
        assert_eq!(emphasis.value(0), 0.0);
        emphasis.set_active(3, Some(0));
        assert_eq!(emphasis.active(), Some(0));
        assert_eq!(emphasis.value(0), 0.0, "the fade starts where it was");

        let mut running = true;
        let mut elapsed = 0;
        while running {
            running = emphasis.advance(SECTOR_TRANSITION_MS / 8);
            elapsed += SECTOR_TRANSITION_MS / 8;
        }
        assert_eq!(elapsed, SECTOR_TRANSITION_MS);
        assert_eq!(emphasis.value(0), 1.0);
        assert_eq!(emphasis.value(1), 0.0, "neighbours stay dark");
    }

    #[test]
    fn the_fade_leads_its_linear_equivalent() {
        let mut emphasis = SectorEmphasis::new(1);
        emphasis.set_active(1, Some(0));
        emphasis.advance(SECTOR_TRANSITION_MS / 2);
        assert!(
            emphasis.value(0) > 0.5,
            "half the time must be past half lit"
        );
        assert!(emphasis.value(0) < 1.0);
    }

    #[test]
    fn a_settled_emphasis_reports_no_animation() {
        let mut emphasis = SectorEmphasis::new(2);
        assert!(!emphasis.advance(ANIMATION_FRAME_MS));
        emphasis.set_active(2, Some(1));
        assert!(emphasis.advance(ANIMATION_FRAME_MS));
    }

    #[test]
    fn moving_off_a_sector_fades_it_back_out() {
        let mut emphasis = SectorEmphasis::new(2);
        emphasis.set_active(2, Some(0));
        while emphasis.advance(SECTOR_TRANSITION_MS) {}
        emphasis.set_active(2, Some(1));
        assert_eq!(emphasis.value(0), 1.0, "the fade out starts lit");
        while emphasis.advance(SECTOR_TRANSITION_MS) {}
        assert_eq!(emphasis.value(0), 0.0);
        assert_eq!(emphasis.value(1), 1.0);
    }

    #[test]
    fn re_aiming_mid_fade_carries_on_without_jumping() {
        let mut emphasis = SectorEmphasis::new(2);
        emphasis.set_active(2, Some(0));
        for _ in 0..4 {
            emphasis.advance(ANIMATION_FRAME_MS);
        }
        let lit = emphasis.value(0);
        assert!(lit > 0.0 && lit < 1.0, "still mid-fade, got {lit}");

        emphasis.set_active(2, None);
        assert_eq!(emphasis.value(0), lit, "the value on screen must not jump");
        assert!(emphasis.advance(ANIMATION_FRAME_MS));
    }

    #[test]
    fn hover_leaving_the_ring_finishes_every_fade() {
        let mut emphasis = SectorEmphasis::new(2);
        emphasis.set_active(2, Some(0));
        while emphasis.advance(SECTOR_TRANSITION_MS / 4) {}
        emphasis.set_active(2, None);
        while emphasis.advance(SECTOR_TRANSITION_MS) {}
        assert_eq!(emphasis.value(0), 0.0);
        assert_eq!(emphasis.value(1), 0.0);
    }

    #[test]
    fn a_new_ring_starts_dark() {
        let mut emphasis = SectorEmphasis::new(2);
        emphasis.set_active(2, Some(1));
        emphasis.reset(5);
        assert_eq!(emphasis.active(), None);
        assert_eq!(emphasis.fades.len(), 5);
        assert!((0..5).all(|index| emphasis.value(index) == 0.0));
    }

    #[test]
    fn a_ring_with_a_different_sector_count_starts_dark() {
        let mut emphasis = SectorEmphasis::new(2);
        emphasis.set_active(2, Some(1));
        emphasis.set_active(4, Some(3));
        assert_eq!(emphasis.value(3), 0.0);
        assert!(emphasis.advance(ANIMATION_FRAME_MS));
        while emphasis.advance(SECTOR_TRANSITION_MS) {}
        assert_eq!(emphasis.value(3), 1.0);
        assert_eq!(emphasis.value(1), 0.0);
    }

    #[test]
    fn sectors_off_the_ring_read_as_dark() {
        let emphasis = SectorEmphasis::new(2);
        assert_eq!(emphasis.value(7), 0.0);
    }
}

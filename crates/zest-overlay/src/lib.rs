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

mod icons;

#[cfg(windows)]
mod text;

pub use icons::Icon;

/// Ease-out duration for sector hover transitions.
pub const SECTOR_TRANSITION_MS: u64 = 200;
/// Frame interval the hover fade runs at, ~60fps.
pub const ANIMATION_FRAME_MS: u64 = 16;
pub const OVERLAY_SIZE: f32 = 256.0;
pub const RING_INNER_RADIUS: f32 = 52.0;
pub const RING_OUTER_RADIUS: f32 = 112.0;

/// Where a sector's icon sits along its own radius, and how large it is drawn.
pub const ICON_RADIUS: f32 = 94.0;
pub const ICON_SIZE: f32 = 18.0;
/// Where a sector's label sits, closer in than the icon so the two read as one
/// stack rather than two things competing for the same band.
pub const LABEL_RADIUS: f32 = 68.0;
/// The biggest a label is ever drawn. Ring 1 has two sectors and can afford it.
pub const LABEL_MAX_SIZE: f32 = 13.0;
/// …and the smallest, past which a label is trimmed rather than shrunk further.
pub const LABEL_MIN_SIZE: f32 = 8.0;
/// Padding either side of a label's box, so it never touches a sector border.
pub const LABEL_PADDING: f32 = 6.0;
/// How far inside the inner disc the centre's content sits, as a fraction of the
/// radius. The disc is stroked 1.5px, so the content has to clear it.
pub const CENTRE_INSET: f32 = 0.62;

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

/// The gradient's colour at `t` across its 0.0..=1.0 span. The overlay paints the
/// ramp as flat bands, because a linear gradient brush does not ramp on the
/// GDI-compatible render target, so this is what turns the stop list back into a
/// gradient one band at a time.
pub fn ramp_color(stops: &[Rgba], t: f32) -> Rgba {
    let opaque = Rgba {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 1.0,
    };
    let Some(first) = stops.first().copied() else {
        return opaque;
    };
    if stops.len() < 2 {
        return first;
    }
    let last = stops.len() - 1;
    let scaled = t.clamp(0.0, 1.0) * last as f32;
    let lower = (scaled.floor() as usize).min(last);
    let upper = (lower + 1).min(last);
    let mix = scaled - lower as f32;
    let (from, to) = (stops[lower], stops[upper]);
    let lerp = |a: f32, b: f32| a + (b - a) * mix;
    Rgba {
        r: lerp(from.r, to.r),
        g: lerp(from.g, to.g),
        b: lerp(from.b, to.b),
        a: lerp(from.a, to.a),
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
    pub icon: Icon,
    /// Center angle in radians, starting at -π/2.
    pub center: f64,
    /// Angular width in radians.
    pub width: f64,
}

/// What a sector carries before it is laid out around the circle: the label the
/// menu node names and the icon that node's action implies.
#[derive(Debug, Clone, PartialEq)]
pub struct SectorContent {
    pub label: String,
    pub icon: Icon,
}

impl SectorContent {
    pub fn new(label: impl Into<String>, icon: Icon) -> Self {
        Self {
            label: label.into(),
            icon,
        }
    }
}

/// Evenly fan `content` around a full circle starting at top (-90°).
pub fn layout_sectors(content: &[SectorContent]) -> Vec<Sector> {
    let n = content.len() as f64;
    if n == 0.0 {
        return vec![];
    }
    content
        .iter()
        .enumerate()
        .map(|(i, item)| Sector {
            label: item.label.clone(),
            icon: item.icon,
            center: -PI / 2.0 + (i as f64 / n) * TAU,
            width: TAU / n,
        })
        .collect()
}

/// The widest ring Zest's menu can currently produce. The label size table is
/// written against this, so a new conversion target that made a ring wider would
/// silently fall through to the floor size.
pub const WIDEST_RING: usize = 8;

/// The label size for a ring of `sectors` sectors.
///
/// Every sector shares one size, so the whole ring reads as a single scale
/// rather than a ring of mismatched labels. More sectors means a thinner slice
/// of arc at the label's radius, so the size steps down with it.
pub fn label_font_size(sectors: usize) -> f32 {
    match sectors {
        0 => LABEL_MIN_SIZE,
        1..=2 => 13.0,
        3..=4 => 11.5,
        5..=6 => 10.5,
        7..=WIDEST_RING => 9.5,
        _ => LABEL_MIN_SIZE,
    }
}

/// The width a sector's label may occupy: the arc it sits on, less padding,
/// capped at the ring's own chord so a lone sector cannot ask for a box wider
/// than the dial.
///
/// A label is drawn horizontally, so its box is as wide as the arc under it.
/// Without this a long label spills into the neighbouring sector and the ring
/// becomes unreadable at exactly the moment the user is choosing.
pub fn label_box_width(sectors: usize) -> f32 {
    let half = RING_OUTER_RADIUS * RING_OUTER_RADIUS - LABEL_RADIUS * LABEL_RADIUS;
    let chord = 2.0 * half.max(0.0).sqrt();
    if sectors == 0 {
        return chord;
    }
    let arc = LABEL_RADIUS * (TAU as f32 / sectors as f32);
    (arc - LABEL_PADDING * 2.0).clamp(LABEL_MIN_SIZE, chord)
}

/// Shrink `size` until `measured` fits `available`, never past
/// [`LABEL_MIN_SIZE`].
///
/// DirectWrite trims whatever still does not fit, so the floor is a legibility
/// limit rather than a correctness one — going smaller buys nothing.
pub fn fitted_label_size(size: f32, measured: f32, available: f32) -> f32 {
    if measured <= 0.0 || measured <= available {
        return size;
    }
    (size * available / measured).max(LABEL_MIN_SIZE)
}

/// Ink for chrome drawn on an unlit sector.
pub const CHROME_INK: Rgba = Rgba {
    r: 0.96,
    g: 0.97,
    b: 1.0,
    a: 1.0,
};

/// Ink for chrome drawn on the bright end of a lit ramp.
pub const CHROME_INK_ON_BRIGHT: Rgba = Rgba {
    r: 0.06,
    g: 0.06,
    b: 0.08,
    a: 1.0,
};

/// The ramp luminance above which chrome goes dark. Rec. 709 weights, the same
/// ones the pixel read-backs use, so "bright" means the same thing in both.
const BRIGHT_LUMINANCE: f32 = 0.55;

/// Where along the ring's radius a piece of chrome sits, as a 0.0..=1.0 position
/// across the ramp. The ramp runs inner edge to outer edge, so this is the same
/// fraction the bands use.
fn ramp_fraction(radius: f32) -> f32 {
    ((radius - RING_INNER_RADIUS) / (RING_OUTER_RADIUS - RING_INNER_RADIUS)).clamp(0.0, 1.0)
}

/// Relative luminance of a colour, Rec. 709.
fn luminance(color: Rgba) -> f32 {
    0.2126 * color.r + 0.7152 * color.g + 0.0722 * color.b
}

/// The box the centre's content is drawn into: the largest square that clears
/// the disc's border.
///
/// The preview is centre-cropped to a square before it reaches the renderer, so
/// this is one shape for both a preview and a badge.
pub fn centre_box() -> OverlayBox {
    let dial = OVERLAY_SIZE / 2.0;
    let half = RING_INNER_RADIUS * CENTRE_INSET;
    OverlayBox::new(dial - half, dial - half, dial + half, dial + half)
}

/// The ink a piece of chrome should be drawn in, at `radius`, while the sector
/// underneath it is lit to `emphasis`.
///
/// A lit sector's ramp runs from the inner edge to the outer one, and the
/// gradient is the user's to choose — it can be bright or it can be nearly
/// black. Dark ink on a bright ramp is readable and light ink is not; dark ink on
/// a dark ramp is the reverse, and is how a label disappears entirely. So the
/// ink is chosen from the colour the ramp actually puts under this piece of
/// chrome, and eased in with the hover like the ramp itself.
pub fn chrome_ink(stops: &[Rgba], radius: f32, emphasis: f32) -> Rgba {
    let underneath = ramp_color(stops, ramp_fraction(radius));
    let target = if luminance(underneath) > BRIGHT_LUMINANCE {
        CHROME_INK_ON_BRIGHT
    } else {
        CHROME_INK
    };
    CHROME_INK.lerp(target, emphasis.clamp(0.0, 1.0))
}

/// An axis-aligned box in overlay coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OverlayBox {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl OverlayBox {
    pub fn new(left: f32, top: f32, right: f32, bottom: f32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    pub fn center(&self) -> (f32, f32) {
        (
            (self.left + self.right) / 2.0,
            (self.top + self.bottom) / 2.0,
        )
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.left && x <= self.right && y >= self.top && y <= self.bottom
    }

    /// Whether every corner of `self` is inside the overlay's square.
    pub fn inside_overlay(&self) -> bool {
        let extent = OVERLAY_SIZE;
        self.left >= 0.0 && self.top >= 0.0 && self.right <= extent && self.bottom <= extent
    }
}

/// Where a sector's icon and its label sit: `(icon, label)`, both in overlay
/// coordinates.
///
/// Kept out of the renderer so the placement can be asserted without a render
/// target. The icon goes out near the rim and the label in nearer the centre, so
/// the pair reads as one stack along the sector's own radius rather than two
/// things competing for the same band.
pub fn sector_chrome(sector: &Sector, sectors: usize) -> (OverlayBox, OverlayBox) {
    let dial = OVERLAY_SIZE / 2.0;
    let (sin, cos) = (sector.center.sin() as f32, sector.center.cos() as f32);
    let on_radius = |radius: f32| (dial + radius * cos, dial + radius * sin);

    let (icon_x, icon_y) = on_radius(ICON_RADIUS);
    let icon = OverlayBox::new(
        icon_x - ICON_SIZE / 2.0,
        icon_y - ICON_SIZE / 2.0,
        icon_x + ICON_SIZE / 2.0,
        icon_y + ICON_SIZE / 2.0,
    );

    let size = label_font_size(sectors);
    let (label_x, label_y) = on_radius(LABEL_RADIUS);
    let half = label_box_width(sectors) / 2.0;
    let label = OverlayBox::new(
        label_x - half,
        label_y - size,
        label_x + half,
        label_y + size,
    );
    (icon, label)
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
        let content: Vec<SectorContent> = self
            .current()
            .iter()
            .map(|node| SectorContent {
                label: node.label.clone(),
                icon: icons::icon_for(node),
            })
            .collect();
        self.sectors = layout_sectors(&content);
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

/// What the ring's centre shows: a preview of what the user picked.
///
/// The overlay is deliberately not told anything about files beyond this — it
/// cannot act on a selection, so all it needs is enough to draw the middle of
/// the dial.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum CentreBadge {
    /// Nothing is selected, or the app could not read the selection.
    #[default]
    Nothing,
    /// One file. `preview` is the decoded image when the file is one this
    /// platform can decode, and `None` for everything else — the renderer then
    /// shows `text`, which is the file's extension.
    One { text: String, preview: bool },
    /// More than one file: the count.
    Many { count: usize },
}

impl CentreBadge {
    /// The badge for a resolved selection.
    pub fn for_selection(selection: &zest_core::Selection) -> Self {
        match selection.files.len() {
            0 => Self::Nothing,
            1 => {
                let path = &selection.files[0];
                let text = path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .filter(|ext| !ext.is_empty())
                    .map(|ext| ext.to_ascii_uppercase())
                    .unwrap_or_else(|| "FILE".to_string());
                let preview =
                    zest_core::file_kind::classify_path(path) == zest_core::FileKind::Image;
                Self::One { text, preview }
            }
            count => Self::Many { count },
        }
    }

    /// The text the centre draws, if any.
    pub fn text(&self) -> Option<String> {
        match self {
            CentreBadge::Nothing => None,
            CentreBadge::One { text, .. } => Some(text.clone()),
            CentreBadge::Many { count } => Some(count.to_string()),
        }
    }

    /// The file to preview, when the badge is a single decodable image. Read by
    /// the overlay thread, which is where the decode happens: the app's event
    /// loop hands over a path and never waits on pixel work.
    pub fn preview_path<'a>(
        &self,
        selection: &'a zest_core::Selection,
    ) -> Option<&'a std::path::Path> {
        match self {
            CentreBadge::One { preview: true, .. } => selection.files.first().map(|p| p.as_path()),
            _ => None,
        }
    }
}

/// What the app hands over for the next showing: the ring to draw, the
/// active-sector gradient the user configured, and what the centre shows.
pub(crate) struct PendingOverlay {
    pub(crate) ring: RingModel,
    pub(crate) gradient: Gradient,
    pub(crate) centre: CentreBadge,
    /// The single image to preview in the centre, if there is one. The overlay
    /// owns decoding so the file is only read when the ring actually appears.
    pub(crate) preview: Option<std::path::PathBuf>,
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
                category: None,
                children: child_labels
                    .iter()
                    .map(|child| MenuNode {
                        label: (*child).to_string(),
                        action: Some(MenuAction::Convert {
                            ext: (*child).to_string(),
                        }),
                        category: None,
                        children: Vec::new(),
                    })
                    .collect(),
            })
            .collect()
    }

    /// Ring-1 categories in `menu` carry a category, which is what the overlay
    /// reads to pick their icon. Tests only need the ring shape, so this keeps
    /// the fixtures honest about which half of `MenuNode` they are filling in.
    fn categories(
        labels: &[(&str, zest_core::ActionCategory)],
        children: &[&[&str]],
    ) -> Vec<MenuNode> {
        menu(
            &labels.iter().map(|(label, _)| *label).collect::<Vec<_>>(),
            children,
        )
        .into_iter()
        .zip(labels)
        .map(|(mut node, (_, category))| {
            node.category = Some(*category);
            node
        })
        .collect()
    }

    const RING_ONE: &[(&str, zest_core::ActionCategory)] = &[
        ("Convert", zest_core::ActionCategory::Convert),
        ("Archive", zest_core::ActionCategory::Archive),
    ];

    fn sectors(labels: &[&str]) -> Vec<SectorContent> {
        labels
            .iter()
            .map(|label| SectorContent::new(*label, Icon::File))
            .collect()
    }

    #[test]
    fn three_sectors_cover_circle() {
        let laid_out = layout_sectors(&sectors(&["A", "B", "C"]));
        assert_eq!(laid_out.len(), 3);
        assert!(hit_test(&laid_out, 0.0).is_some());
    }

    #[test]
    fn first_sector_starts_at_top() {
        let laid_out = layout_sectors(&sectors(&["A"]));
        assert_eq!(laid_out[0].center, -PI / 2.0);
        assert_eq!(hit_test(&laid_out, -PI / 2.0), Some(0));
    }

    #[test]
    fn point_hit_test_respects_center_dead_zone() {
        let laid_out = layout_sectors(&sectors(&["A", "B"]));
        assert_eq!(hit_test_at_point(&laid_out, (128.0, 128.0)), None);
        assert_eq!(hit_test_at_point(&laid_out, (128.0, 28.0)), Some(0));
    }

    #[test]
    fn click_expands_then_yields_the_leaf_action() {
        let mut ring = RingModel::new(categories(RING_ONE, &[&["png"], &[]]));
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
        let mut ring = RingModel::new(categories(RING_ONE, &[&["png"], &[]]));
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
    fn every_sector_keeps_the_icon_its_node_implies() {
        let mut ring = RingModel::new(categories(RING_ONE, &[&["png"], &[]]));
        assert_eq!(
            ring.sectors.iter().map(|s| s.icon).collect::<Vec<_>>(),
            vec![Icon::Repeat, Icon::Archive]
        );
        ring.click(0);
        assert_eq!(
            ring.sectors.iter().map(|s| s.icon).collect::<Vec<_>>(),
            vec![Icon::Image]
        );
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
    fn the_ramp_runs_from_the_first_stop_to_the_last() {
        let stops = vec![
            Rgba {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 1.0,
            },
            Rgba {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0,
            },
        ];
        assert!(ramp_color(&stops, 0.0).r < 0.001);
        assert!((ramp_color(&stops, 0.5).r - 0.5).abs() < 0.001);
        assert!(ramp_color(&stops, 1.0).r > 0.999);
    }

    #[test]
    fn the_ramp_clamps_outside_its_span() {
        let stops = vec![
            Rgba {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 1.0,
            },
            Rgba {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0,
            },
        ];
        assert!(ramp_color(&stops, -1.0).r < 0.001);
        assert!(ramp_color(&stops, 2.0).r > 0.999);
    }

    #[test]
    fn a_three_stop_ramp_passes_through_the_middle_one() {
        let stops = vec![
            Rgba {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 1.0,
            },
            Rgba {
                r: 1.0,
                g: 0.0,
                b: 0.0,
                a: 1.0,
            },
            Rgba {
                r: 0.0,
                g: 0.0,
                b: 1.0,
                a: 1.0,
            },
        ];
        let middle = ramp_color(&stops, 0.5);
        assert!(
            (middle.r - 1.0).abs() < 0.001 && middle.b < 0.001,
            "the halfway band should be the middle stop, read {middle:?}"
        );
    }

    #[test]
    fn a_single_stop_ramp_is_that_colour_everywhere() {
        let stops = vec![Rgba {
            r: 0.25,
            g: 0.5,
            b: 0.75,
            a: 1.0,
        }];
        assert_eq!(ramp_color(&stops, 0.0), stops[0]);
        assert_eq!(ramp_color(&stops, 1.0), stops[0]);
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

    #[test]
    fn a_bigger_ring_draws_its_labels_smaller() {
        let sizes: Vec<f32> = (1..=WIDEST_RING).map(label_font_size).collect();
        assert!(
            sizes.windows(2).all(|pair| pair[0] >= pair[1]),
            "label size should not grow with the sector count, read {sizes:?}"
        );
        assert_eq!(
            sizes[0], LABEL_MAX_SIZE,
            "two sectors get the biggest label"
        );
        assert!(sizes.iter().all(|size| *size >= LABEL_MIN_SIZE));
    }

    #[test]
    fn an_empty_ring_has_no_label_to_size() {
        assert_eq!(label_font_size(0), LABEL_MIN_SIZE);
        assert!(label_box_width(0) > 0.0);
    }

    #[test]
    fn a_narrower_ring_gives_its_labels_less_room() {
        let widths: Vec<f32> = (1..=WIDEST_RING).map(label_box_width).collect();
        assert!(
            widths.windows(2).all(|pair| pair[0] >= pair[1]),
            "the arc under a label shrinks with the sector count, read {widths:?}"
        );
        // Past the chord cap it does shrink strictly, so the table is doing work
        // rather than every ring getting the cap.
        assert!(
            widths[1..].windows(2).all(|pair| pair[0] > pair[1]),
            "read {widths:?}"
        );
    }

    /// A lone sector is the whole ring, so its arc is the entire circumference
    /// and its label box has to be capped at the dial's own chord or it runs off
    /// both edges.
    #[test]
    fn a_lone_sectors_label_box_stays_inside_the_ring() {
        let chord =
            2.0 * (RING_OUTER_RADIUS * RING_OUTER_RADIUS - LABEL_RADIUS * LABEL_RADIUS).sqrt();
        let width = label_box_width(1);
        assert!(
            (width - chord).abs() < 0.01,
            "one sector should be capped at the chord, read {width} against {chord}"
        );
        // The box has to stay on the 256px dial when centred on its radius.
        let centre = OVERLAY_SIZE / 2.0;
        let (_, label) = sector_chrome(&layout_sectors(&sectors(&["Everything"]))[0], 1);
        assert!(
            label.left >= 0.0 && label.right <= OVERLAY_SIZE,
            "a lone sector's label box left the dial, read {label:?}"
        );
        assert!(label.center().0 == centre);
    }

    /// The label has to fit the arc it sits on, or it spills into the
    /// neighbouring sector and the ring becomes unreadable. This bounds the
    /// worst case with a conservative 0.62em per character, which is wider than
    /// Segoe UI's average advance.
    #[test]
    fn every_label_size_fits_the_arc_it_is_drawn_on() {
        const ADVANCE_EM: f32 = 0.62;
        // The longest label any ring can currently produce.
        const LONGEST: &str = "tar.gz";
        for sectors in 1..=WIDEST_RING {
            let size = label_font_size(sectors);
            let needed = LONGEST.chars().count() as f32 * ADVANCE_EM * size;
            let available = label_box_width(sectors);
            assert!(
                needed <= available,
                "{sectors} sectors: {LONGEST:?} needs {needed:.1}px of {available:.1}px"
            );
        }
    }

    /// The size table tops out at [`WIDEST_RING`]. If a new conversion target made
    /// a ring wider than that, labels would quietly drop to the floor size and
    /// start trimming, and nothing else would notice.
    #[test]
    fn no_ring_zest_can_build_is_wider_than_the_size_table() {
        let widest = [
            zest_core::FileKind::Image,
            zest_core::FileKind::Video,
            zest_core::FileKind::Audio,
            zest_core::FileKind::TextData,
        ]
        .into_iter()
        .map(|kind| zest_core::convert_targets(kind).len())
        // A mixed selection gets the Archive category, whose fan-out is the
        // other ring worth measuring.
        .chain([
            zest_core::menu_for_selection(&zest_core::Selection::new(vec![
                "a.png".into(),
                "b.mp3".into(),
            ]))[0]
                .children
                .len(),
        ])
        .max()
        .expect("the menu has targets");
        assert_eq!(
            widest, WIDEST_RING,
            "the widest ring changed; label_font_size needs a size for it"
        );
    }

    #[test]
    fn a_label_that_fits_is_left_at_its_size() {
        assert_eq!(fitted_label_size(13.0, 40.0, 100.0), 13.0);
        assert_eq!(fitted_label_size(13.0, 0.0, 100.0), 13.0);
    }

    #[test]
    fn a_label_that_overflows_is_shrunk_to_fit() {
        // Measured one and a half times the room available: two thirds the size.
        assert!((fitted_label_size(13.0, 60.0, 40.0) - 8.666_667).abs() < 0.001);
    }

    /// Shrinking past the floor buys nothing, because DirectWrite trims what
    /// still does not fit.
    #[test]
    fn a_label_that_never_shrinks_below_the_legibility_floor() {
        assert_eq!(fitted_label_size(13.0, 10_000.0, 1.0), LABEL_MIN_SIZE);
    }

    /// A bright gradient is the common case: the ring's default is orange, and
    /// dark ink is what stays readable on it.
    #[test]
    fn chrome_goes_dark_over_a_bright_ramp() {
        let bright = vec![
            Rgba {
                r: 1.0,
                g: 0.6,
                b: 0.2,
                a: 1.0,
            },
            Rgba {
                r: 1.0,
                g: 0.9,
                b: 0.7,
                a: 1.0,
            },
        ];
        let ink = chrome_ink(&bright, LABEL_RADIUS, 1.0);
        assert!(
            ink.r < 0.2 && ink.g < 0.2 && ink.b < 0.2,
            "a bright ramp should carry dark chrome, read {ink:?}"
        );
    }

    /// …and a dark one must not swallow it. The gradient is the user's to pick,
    /// so a near-black ramp is reachable, and near-black ink on near-black is how
    /// the labels silently vanished.
    #[test]
    fn chrome_stays_light_over_a_dark_ramp() {
        let dark = vec![
            Rgba {
                r: 0.05,
                g: 0.05,
                b: 0.08,
                a: 1.0,
            },
            Rgba {
                r: 0.10,
                g: 0.10,
                b: 0.14,
                a: 1.0,
            },
        ];
        for radius in [LABEL_RADIUS, ICON_RADIUS] {
            let ink = chrome_ink(&dark, radius, 1.0);
            assert!(
                luminance(ink) > 0.8,
                "a dark ramp should carry light chrome at r={radius}, read {ink:?}"
            );
        }
    }

    /// Chrome must be readable wherever on the ramp it sits, so the choice is
    /// made per radius: a ramp that is dark at the label and bright at the icon
    /// needs opposite ink for the two.
    #[test]
    fn the_ink_is_chosen_for_the_colour_under_each_piece_of_chrome() {
        let dark_inner = vec![
            Rgba {
                r: 0.02,
                g: 0.02,
                b: 0.02,
                a: 1.0,
            },
            Rgba {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0,
            },
        ];
        let label = chrome_ink(&dark_inner, LABEL_RADIUS, 1.0);
        let icon = chrome_ink(&dark_inner, ICON_RADIUS, 1.0);
        assert!(
            luminance(label) > 0.8,
            "the label sits over the dark end, so it stays light, read {label:?}"
        );
        assert!(
            luminance(icon) < 0.2,
            "the icon sits over the bright end, so it goes dark, read {icon:?}"
        );
    }

    /// An unlit sector is dark whatever the gradient says, so chrome starts light
    /// and only flips as the ramp fades in.
    #[test]
    fn chrome_starts_light_and_eases_to_whatever_the_ramp_needs() {
        let dark = vec![Rgba {
            r: 0.02,
            g: 0.02,
            b: 0.02,
            a: 1.0,
        }];
        let resting = chrome_ink(&dark, LABEL_RADIUS, 0.0);
        assert_eq!(resting, CHROME_INK, "nothing is lit yet");
        // A bright ramp is the case where the ink actually has to change.
        let bright = vec![Rgba {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 1.0,
        }];
        let half = chrome_ink(&bright, LABEL_RADIUS, 0.5);
        assert!(
            luminance(half) < luminance(CHROME_INK)
                && luminance(half) > luminance(CHROME_INK_ON_BRIGHT),
            "halfway through the fade the ink should be between the two, read {half:?}"
        );
    }

    /// A radius outside the ring clamps to the ends of the ramp.
    #[test]
    fn a_radius_outside_the_ring_clamps_to_the_ends_of_the_ramp() {
        let stops = vec![
            Rgba {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 1.0,
            },
            Rgba {
                r: 1.0,
                g: 1.0,
                b: 1.0,
                a: 1.0,
            },
        ];
        assert_eq!(ramp_fraction(RING_INNER_RADIUS), 0.0);
        assert_eq!(ramp_fraction(RING_OUTER_RADIUS), 1.0);
        assert_eq!(ramp_fraction(-100.0), 0.0);
        assert_eq!(ramp_fraction(1_000.0), 1.0);
        let _ = chrome_ink(&stops, 1_000.0, 1.0);
    }

    /// Everything the centre draws has to stay on the dial. One shape for a
    /// preview and a badge, because the preview is squared before it gets here.
    #[test]
    fn the_centre_box_is_the_disc_square_for_everything_drawn_there() {
        let box_ = centre_box();
        let width = box_.right - box_.left;
        let height = box_.bottom - box_.top;
        assert!(
            (width - height).abs() < 0.01,
            "the centre box should be square"
        );
        assert!(
            width <= RING_INNER_RADIUS * 2.0,
            "the centre box would overrun the disc, read {width}"
        );
        assert!(box_.inside_overlay());
        assert!(
            (box_.center().0 - OVERLAY_SIZE / 2.0).abs() < 0.01,
            "the centre box should be on the dial"
        );
    }
    #[test]
    fn one_file_reads_as_its_extension() {
        let badge =
            CentreBadge::for_selection(&zest_core::Selection::new(vec!["photo.PNG".into()]));
        assert_eq!(
            badge,
            CentreBadge::One {
                text: "PNG".into(),
                preview: true
            }
        );
        assert_eq!(badge.text(), Some("PNG".to_string()));
    }

    #[test]
    fn a_file_with_no_extension_falls_back_to_a_word() {
        let badge = CentreBadge::for_selection(&zest_core::Selection::new(vec!["README".into()]));
        assert_eq!(
            badge.text(),
            Some("FILE".to_string()),
            "a dotfile or extensionless file still needs a label"
        );
        assert_eq!(
            badge,
            CentreBadge::One {
                text: "FILE".into(),
                preview: false
            }
        );
    }

    #[test]
    fn a_file_zest_cannot_preview_is_still_labelled() {
        let badge =
            CentreBadge::for_selection(&zest_core::Selection::new(vec!["bundle.zip".into()]));
        assert_eq!(badge.text(), Some("ZIP".to_string()));
        assert!(matches!(badge, CentreBadge::One { preview: false, .. }));
    }

    #[test]
    fn several_files_read_as_a_count() {
        let badge = CentreBadge::for_selection(&zest_core::Selection::new(vec![
            "a.png".into(),
            "b.png".into(),
            "c.png".into(),
        ]));
        assert_eq!(badge, CentreBadge::Many { count: 3 });
        assert_eq!(badge.text(), Some("3".to_string()));
    }

    #[test]
    fn an_empty_selection_has_no_badge() {
        let badge = CentreBadge::for_selection(&zest_core::Selection::default());
        assert_eq!(badge, CentreBadge::Nothing);
        assert_eq!(badge.text(), None);
    }

    #[test]
    fn a_preview_is_only_offered_for_a_single_image() {
        let image = zest_core::Selection::new(vec!["a.png".into()]);
        let archive = zest_core::Selection::new(vec!["a.zip".into()]);
        let many = zest_core::Selection::new(vec!["a.png".into(), "b.png".into()]);
        assert_eq!(
            CentreBadge::for_selection(&image).preview_path(&image),
            Some(std::path::Path::new("a.png"))
        );
        assert_eq!(
            CentreBadge::for_selection(&archive).preview_path(&archive),
            None
        );
        assert_eq!(CentreBadge::for_selection(&many).preview_path(&many), None);
    }

    #[test]
    fn a_sectors_icon_sits_on_its_own_radius() {
        let laid_out = layout_sectors(&sectors(&["A", "B", "C"]));
        for sector in &laid_out {
            let (icon, _) = sector_chrome(sector, laid_out.len());
            let (x, y) = icon.center();
            let dial = OVERLAY_SIZE / 2.0;
            let radius = (x - dial).hypot(y - dial);
            assert!(
                (radius - ICON_RADIUS).abs() < 0.01,
                "the icon should sit at ICON_RADIUS, read {radius} for {:?}",
                sector.label
            );
            let angle = f64::from((y - dial).atan2(x - dial));
            assert!(
                (angle - sector.center).abs() < 1e-6,
                "the icon should sit on the sector's own angle, read {angle} against {}",
                sector.center
            );
        }
    }

    /// The label box is a centre-and-clip rectangle, so its corners are allowed to
    /// fall outside the annulus where the ring actually shows. What must hold is
    /// that the box stays on the dial, that the text is centred in the visible
    /// band, and that the box never reaches past the outer rim.
    #[test]
    fn a_sectors_chrome_stays_on_the_dial_and_in_the_band() {
        let dial = OVERLAY_SIZE / 2.0;
        let rim =
            2.0 * (RING_OUTER_RADIUS * RING_OUTER_RADIUS - LABEL_RADIUS * LABEL_RADIUS).sqrt();
        for count in 1..=WIDEST_RING {
            let labels: Vec<SectorContent> = (0..count)
                .map(|index| SectorContent::new(format!("{index}"), Icon::File))
                .collect();
            let laid_out = layout_sectors(&labels);
            for sector in &laid_out {
                let (icon, label) = sector_chrome(sector, count);
                for (what, box_) in [("icon", icon), ("label", label)] {
                    assert!(
                        box_.inside_overlay(),
                        "{count} sectors: the {what} for {:?} left the dial, read {box_:?}",
                        sector.label
                    );
                }
                // The icon is a small centred mark, so all of it has to be in the
                // band; the label is only bounded by the rim at its widest.
                for (cx, cy) in box_corners(&icon) {
                    let radius = (cx - dial).hypot(cy - dial);
                    assert!(
                        (RING_INNER_RADIUS - 0.01..=RING_OUTER_RADIUS + 0.01).contains(&radius),
                        "{count} sectors: the icon for {:?} reaches r={radius:.1}, outside \
                         {RING_INNER_RADIUS}..={RING_OUTER_RADIUS}",
                        sector.label
                    );
                }
                assert!(
                    label.right - label.left <= rim + 0.01,
                    "{count} sectors: the label for {:?} is wider than the ring's chord",
                    sector.label
                );
                let (lx, ly) = label.center();
                let radius = (lx - dial).hypot(ly - dial);
                assert!(
                    (radius - LABEL_RADIUS).abs() < 0.01,
                    "{count} sectors: the label for {:?} should be centred on LABEL_RADIUS, read {radius:.1}",
                    sector.label
                );
            }
        }
    }

    /// Two sectors on one ring put their labels at opposite sides of the dial.
    /// If those boxes overlapped, both labels would be clipped to the same arc.
    #[test]
    fn neighbouring_labels_do_not_claim_the_same_arc() {
        let laid_out = layout_sectors(&sectors(&["A", "B"]));
        let first = sector_chrome(&laid_out[0], 2).1;
        let second = sector_chrome(&laid_out[1], 2).1;
        let overlaps = first.left < second.right
            && second.left < first.right
            && first.top < second.bottom
            && second.top < first.bottom;
        assert!(
            !overlaps,
            "the two labels overlap: {first:?} and {second:?}"
        );
    }

    /// The eight-sector Convert ring is the widest Zest builds, so it is the case
    /// where a label is most likely to run into its neighbours.
    #[test]
    fn the_widest_ring_zest_builds_keeps_its_labels_apart() {
        let labels: Vec<&str> = zest_core::convert_targets(zest_core::FileKind::TextData).to_vec();
        assert_eq!(labels.len(), WIDEST_RING, "the text ring is the widest one");
        let laid_out = layout_sectors(
            &labels
                .iter()
                .map(|label| SectorContent::new(*label, Icon::Music))
                .collect::<Vec<_>>(),
        );
        for pair in laid_out.windows(2) {
            let first = sector_chrome(&pair[0], laid_out.len()).1;
            let second = sector_chrome(&pair[1], laid_out.len()).1;
            let overlaps = first.left < second.right
                && second.left < first.right
                && first.top < second.bottom
                && second.top < first.bottom;
            assert!(
                !overlaps,
                "{:?} and {:?} overlap: {first:?} and {second:?}",
                pair[0].label, pair[1].label
            );
        }
    }

    /// The four corners of a box, for checking where a box reaches.
    fn box_corners(box_: &OverlayBox) -> [(f32, f32); 4] {
        [
            (box_.left, box_.top),
            (box_.right, box_.top),
            (box_.left, box_.bottom),
            (box_.right, box_.bottom),
        ]
    }
}

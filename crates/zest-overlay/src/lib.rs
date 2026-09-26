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
use zest_core::{MenuAction, MenuNode};

/// Ease-out duration for sector hover transitions.
pub const SECTOR_TRANSITION_MS: u64 = 200;
pub const OVERLAY_SIZE: f32 = 256.0;
pub const RING_INNER_RADIUS: f32 = 52.0;
pub const RING_OUTER_RADIUS: f32 = 112.0;

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
}

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
use zest_core::menu::ActionCategory;

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

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct RingModel {
    pub(crate) sectors: Vec<Sector>,
    pub(crate) children: Vec<Vec<Sector>>,
}

impl RingModel {
    pub(crate) fn new(labels: &[String], children: &[Vec<String>]) -> Option<Self> {
        if labels.len() != children.len() {
            return None;
        }
        Some(Self {
            sectors: layout_sectors(labels),
            children: children
                .iter()
                .map(|labels| layout_sectors(labels))
                .collect(),
        })
    }

    pub(crate) fn hit_test(&self, point: (f32, f32)) -> Option<usize> {
        hit_test_at_point(&self.sectors, point)
    }

    pub(crate) fn expand(&mut self, index: usize) -> bool {
        let Some(next) = self
            .children
            .get(index)
            .filter(|sectors| !sectors.is_empty())
            .cloned()
        else {
            return false;
        };
        self.sectors = next;
        self.children.clear();
        true
    }
}

/// Ring-1 labels for the overlay, in display order.
pub fn ring_labels(cats: &[ActionCategory]) -> Vec<String> {
    cats.iter()
        .map(|c| match c {
            ActionCategory::Convert => "Convert".to_string(),
            ActionCategory::Archive => "Archive".to_string(),
            ActionCategory::Extract => "Extract".to_string(),
        })
        .collect()
}

#[cfg(windows)]
mod win;

#[cfg(windows)]
pub use win::Overlay;

#[cfg(test)]
mod tests {
    use super::*;

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
    fn ring_expands_once() {
        let mut ring = RingModel::new(
            &["Convert".into(), "Archive".into()],
            &[vec!["png".into()], vec![]],
        )
        .unwrap();
        assert!(ring.expand(0));
        assert_eq!(ring.sectors[0].label, "png");
        assert!(!ring.expand(0));
    }

    #[test]
    fn empty_has_no_hit() {
        assert_eq!(hit_test(&[], 1.0), None);
    }
}

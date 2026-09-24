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

use std::f64::consts::TAU;
use zest_core::menu::ActionCategory;

/// Ease-out duration for sector hover transitions.
pub const SECTOR_TRANSITION_MS: u64 = 200;

#[derive(Debug, Clone)]
pub struct Sector {
    pub label: String,
    /// Center angle in radians [0, TAU).
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
            center: (i as f64 / n) * TAU,
            width: TAU / n,
        })
        .collect()
}

/// Hit-test a cursor angle (radians) against sectors. `None` = center dead-zone.
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
    fn empty_has_no_hit() {
        assert_eq!(hit_test(&[], 1.0), None);
    }
}

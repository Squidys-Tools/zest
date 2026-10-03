//! Lucide icons as vector geometry for the ring.
//!
//! The icons are Lucide's, taken verbatim from `lucide-icons/lucide` and
//! transcribed from their `d` attributes into [`PathOp`] runs: a 24×24 viewbox,
//! stroked at `stroke-width="2"` with round caps and joins, which is exactly
//! what a Direct2D stroke with a round style draws.
//!
//! Keeping them as data rather than assets means the renderer needs no SVG
//! parser and no image decoding — [`Shape`] maps onto `ID2D1Factory`'s rounded
//! rectangle and ellipse geometry, and the rest becomes path geometry.
//!
//! Arc-to-cubic conversion is here rather than in the renderer so the geometry
//! maths stays testable without a render target.

use std::f64::consts::PI;

use zest_core::{ActionCategory, FileKind, MenuAction, MenuNode};

/// The Lucide viewbox every icon is authored in.
pub const VIEWBOX: f32 = 24.0;

/// Lucide's `stroke-width`, in viewbox units.
pub const STROKE_WIDTH: f32 = 2.0;

/// One SVG path command, in absolute viewbox coordinates. Relative commands
/// from the source `d` attributes are resolved at transcription time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PathOp {
    MoveTo(f32, f32),
    LineTo(f32, f32),
    /// Horizontal line to an absolute x.
    HorizontalTo(f32),
    /// Vertical line to an absolute y.
    VerticalTo(f32),
    /// Elliptical arc, SVG `A` semantics.
    ArcTo {
        rx: f32,
        ry: f32,
        /// x-axis rotation in degrees.
        rotation: f32,
        large: bool,
        sweep: bool,
        to: (f32, f32),
    },
    /// Quadratic curve, lifted to a cubic by Direct2D.
    QuadTo(f32, f32, f32, f32),
    CubicTo(f32, f32, f32, f32, f32, f32),
    Close,
}

/// One drawable piece of an icon.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shape {
    /// `x`/`y` are the top-left corner.
    RoundedRect {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        radius: f32,
    },
    Ellipse {
        cx: f32,
        cy: f32,
        r: f32,
    },
    Path(&'static [PathOp]),
}

/// The icon set the ring starts with. Chosen to cover every category and every
/// kind of conversion target the menu can currently offer, so no sector is ever
/// left without one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Icon {
    Archive,
    PackageOpen,
    Repeat,
    Image,
    Film,
    Music,
    FileText,
    Folder,
    File,
}

impl Icon {
    /// The icon's shapes, in draw order.
    pub fn shapes(self) -> &'static [Shape] {
        match self {
            Icon::Archive => ARCHIVE,
            Icon::PackageOpen => PACKAGE_OPEN,
            Icon::Repeat => REPEAT,
            Icon::Image => IMAGE,
            Icon::Film => FILM,
            Icon::Music => MUSIC,
            Icon::FileText => FILE_TEXT,
            Icon::Folder => FOLDER,
            Icon::File => FILE,
        }
    }
}

/// The icon a sector should carry. Driven by the node's action or category
/// rather than its label, so a renamed menu still picks the right picture.
pub fn icon_for(node: &MenuNode) -> Icon {
    match (&node.category, &node.action) {
        (Some(ActionCategory::Convert), _) => Icon::Repeat,
        (Some(ActionCategory::Archive), _) => Icon::Archive,
        // The category is "open this package"; its only leaf is "Here", which
        // means this folder, so the leaf gets a folder rather than a package.
        (Some(ActionCategory::Extract), _) => Icon::PackageOpen,
        (None, Some(MenuAction::Extract)) => Icon::Folder,
        (None, Some(MenuAction::Archive { .. })) => Icon::Archive,
        (None, Some(MenuAction::Convert { ext })) => match FileKind::from_extension(ext) {
            FileKind::Image => Icon::Image,
            FileKind::Video => Icon::Film,
            FileKind::Audio => Icon::Music,
            FileKind::TextData => Icon::FileText,
            _ => Icon::File,
        },
        // A node with neither is not something the menu can currently build, but
        // it still has to paint something.
        (None, None) => Icon::File,
    }
}

/// Approximate an SVG elliptical arc with cubics.
///
/// This is the SVG 1.1 appendix F.6 endpoint-to-centre conversion, then the
/// usual split into quarter-turn cubics. Every arc in the Lucide set is a
/// circular corner (`rx == ry`, no rotation), but the general form costs
/// nothing and keeps the transcription honest if a future icon is not.
///
/// `from` is where the previous command left the pen; `to` is the arc's end.
pub fn arc_cubics(
    from: (f32, f32),
    to: (f32, f32),
    rx: f32,
    ry: f32,
    rotation_deg: f32,
    large: bool,
    sweep: bool,
) -> Vec<[(f32, f32); 3]> {
    let (x1, y1) = (f64::from(from.0), f64::from(from.1));
    let (x2, y2) = (f64::from(to.0), f64::from(to.1));
    let (mut rx, mut ry) = (f64::from(rx).abs(), f64::from(ry).abs());
    if x1 == x2 && y1 == y2 {
        return Vec::new();
    }
    // A zero radius degenerates to a straight line, per the spec.
    if rx == 0.0 || ry == 0.0 {
        return vec![[
            (
                from.0 + (to.0 - from.0) / 3.0,
                from.1 + (to.1 - from.1) / 3.0,
            ),
            (
                from.0 + (to.0 - from.0) * 2.0 / 3.0,
                from.1 + (to.1 - from.1) * 2.0 / 3.0,
            ),
            to,
        ]];
    }

    let phi = f64::from(rotation_deg).to_radians();
    let (sin_phi, cos_phi) = phi.sin_cos();
    let (dx, dy) = ((x1 - x2) / 2.0, (y1 - y2) / 2.0);
    let (x1p, y1p) = (cos_phi * dx + sin_phi * dy, -sin_phi * dx + cos_phi * dy);

    // Radii too small to span the endpoints get scaled up until they do.
    let lambda = (x1p * x1p) / (rx * rx) + (y1p * y1p) / (ry * ry);
    if lambda > 1.0 {
        let scale = lambda.sqrt();
        rx *= scale;
        ry *= scale;
    }

    let (rx2, ry2) = (rx * rx, ry * ry);
    let numerator = rx2 * ry2 - rx2 * y1p * y1p - ry2 * x1p * x1p;
    let denominator = rx2 * y1p * y1p + ry2 * x1p * x1p;
    let coefficient = if denominator == 0.0 {
        0.0
    } else {
        let sign = if large != sweep { 1.0 } else { -1.0 };
        sign * (numerator / denominator).max(0.0).sqrt()
    };
    let (cxp, cyp) = (
        coefficient * (rx * y1p / ry),
        coefficient * (-ry * x1p / rx),
    );
    let (cx, cy) = (
        cos_phi * cxp - sin_phi * cyp + (x1 + x2) / 2.0,
        sin_phi * cxp + cos_phi * cyp + (y1 + y2) / 2.0,
    );

    let angle = |ux: f64, uy: f64, vx: f64, vy: f64| {
        let dot = ux * vx + uy * vy;
        let len = (ux * ux + uy * uy).sqrt() * (vx * vx + vy * vy).sqrt();
        let mut theta = (dot / len).clamp(-1.0, 1.0).acos();
        if ux * vy - uy * vx < 0.0 {
            theta = -theta;
        }
        theta
    };
    let (ux, uy) = ((x1p - cxp) / rx, (y1p - cyp) / ry);
    let (vx, vy) = ((-x1p - cxp) / rx, (-y1p - cyp) / ry);
    let start = angle(1.0, 0.0, ux, uy);
    let mut sweep_angle = angle(ux, uy, vx, vy);
    if !sweep && sweep_angle > 0.0 {
        sweep_angle -= 2.0 * PI;
    } else if sweep && sweep_angle < 0.0 {
        sweep_angle += 2.0 * PI;
    }

    // One cubic per quarter turn: the approximation error grows fast past that.
    let segments = (sweep_angle.abs() / (PI / 2.0)).ceil().max(1.0) as usize;
    let delta = sweep_angle / segments as f64;
    // A point on the rotated ellipse at parametric angle `t`, and the curve's
    // tangent there. The control points are offset along the tangent, not around
    // the ellipse — the difference is what keeps the cubic on the arc instead of
    // cutting the corner.
    let point = |t: f64| {
        let (sin_t, cos_t) = t.sin_cos();
        (
            (cx + rx * cos_t * cos_phi - ry * sin_t * sin_phi) as f32,
            (cy + rx * cos_t * sin_phi + ry * sin_t * cos_phi) as f32,
        )
    };
    let tangent = |t: f64| {
        let (sin_t, cos_t) = t.sin_cos();
        (
            (rx * -sin_t * cos_phi - ry * cos_t * sin_phi) as f32,
            (rx * -sin_t * sin_phi + ry * cos_t * cos_phi) as f32,
        )
    };
    let alpha = 4.0 / 3.0 * (delta / 4.0).tan() as f32;
    let mut theta = start;
    let mut cubics = Vec::with_capacity(segments);
    for _ in 0..segments {
        let next = theta + delta;
        let (start_point, start_tangent) = (point(theta), tangent(theta));
        let (end_point, end_tangent) = (point(next), tangent(next));
        cubics.push([
            (
                start_point.0 + alpha * start_tangent.0,
                start_point.1 + alpha * start_tangent.1,
            ),
            (
                end_point.0 - alpha * end_tangent.0,
                end_point.1 - alpha * end_tangent.1,
            ),
            end_point,
        ]);
        theta = next;
    }
    cubics
}

// --- Lucide path data -------------------------------------------------------
//
// Transcribed from the upstream `d` attributes. Relative commands are resolved
// to absolute coordinates; `a` flags keep their large-arc and sweep values.

/// `archive`
static ARCHIVE: &[Shape] = &[
    Shape::RoundedRect {
        x: 2.0,
        y: 3.0,
        width: 20.0,
        height: 5.0,
        radius: 1.0,
    },
    Shape::Path(&[
        PathOp::MoveTo(4.0, 8.0),
        PathOp::VerticalTo(19.0),
        PathOp::ArcTo {
            rx: 2.0,
            ry: 2.0,
            rotation: 0.0,
            large: false,
            sweep: false,
            to: (6.0, 21.0),
        },
        PathOp::HorizontalTo(18.0),
        PathOp::ArcTo {
            rx: 2.0,
            ry: 2.0,
            rotation: 0.0,
            large: false,
            sweep: false,
            to: (20.0, 19.0),
        },
        PathOp::VerticalTo(8.0),
    ]),
    Shape::Path(&[PathOp::MoveTo(10.0, 12.0), PathOp::HorizontalTo(14.0)]),
];

/// `package-open`
static PACKAGE_OPEN: &[Shape] = &[
    Shape::Path(&[PathOp::MoveTo(12.0, 22.0), PathOp::VerticalTo(13.0)]),
    Shape::Path(&[
        PathOp::MoveTo(15.17, 2.21),
        PathOp::ArcTo {
            rx: 1.67,
            ry: 1.67,
            rotation: 0.0,
            large: false,
            sweep: true,
            to: (16.8, 2.21),
        },
        PathOp::LineTo(21.0, 4.57),
        PathOp::ArcTo {
            rx: 1.93,
            ry: 1.93,
            rotation: 0.0,
            large: false,
            sweep: true,
            to: (21.0, 7.93),
        },
        PathOp::LineTo(8.82, 14.79),
        PathOp::ArcTo {
            rx: 1.655,
            ry: 1.655,
            rotation: 0.0,
            large: false,
            sweep: true,
            to: (7.18, 14.79),
        },
        PathOp::LineTo(3.0, 12.43),
        PathOp::ArcTo {
            rx: 1.93,
            ry: 1.93,
            rotation: 0.0,
            large: false,
            sweep: true,
            to: (3.0, 9.07),
        },
        PathOp::Close,
    ]),
    Shape::Path(&[
        PathOp::MoveTo(20.0, 13.0),
        PathOp::VerticalTo(16.87),
        PathOp::ArcTo {
            rx: 2.06,
            ry: 2.06,
            rotation: 0.0,
            large: false,
            sweep: true,
            to: (18.89, 18.7),
        },
        PathOp::LineTo(12.89, 21.78),
        PathOp::ArcTo {
            rx: 1.93,
            ry: 1.93,
            rotation: 0.0,
            large: false,
            sweep: true,
            to: (11.11, 21.78),
        },
        PathOp::LineTo(5.11, 18.7),
        PathOp::ArcTo {
            rx: 2.06,
            ry: 2.06,
            rotation: 0.0,
            large: false,
            sweep: true,
            to: (4.0, 16.87),
        },
        PathOp::VerticalTo(13.0),
    ]),
    Shape::Path(&[
        PathOp::MoveTo(21.0, 12.43),
        PathOp::ArcTo {
            rx: 1.93,
            ry: 1.93,
            rotation: 0.0,
            large: false,
            sweep: false,
            to: (21.0, 9.07),
        },
        PathOp::LineTo(8.83, 2.2),
        PathOp::ArcTo {
            rx: 1.64,
            ry: 1.64,
            rotation: 0.0,
            large: false,
            sweep: false,
            to: (7.2, 2.2),
        },
        PathOp::LineTo(3.0, 4.57),
        PathOp::ArcTo {
            rx: 1.93,
            ry: 1.93,
            rotation: 0.0,
            large: false,
            sweep: false,
            to: (3.0, 7.93),
        },
        PathOp::LineTo(15.18, 14.79),
        PathOp::ArcTo {
            rx: 1.636,
            ry: 1.636,
            rotation: 0.0,
            large: false,
            sweep: false,
            to: (16.81, 14.79),
        },
        PathOp::Close,
    ]),
];

/// `repeat`
static REPEAT: &[Shape] = &[
    Shape::Path(&[
        PathOp::MoveTo(17.0, 2.0),
        PathOp::LineTo(21.0, 6.0),
        PathOp::LineTo(17.0, 10.0),
    ]),
    Shape::Path(&[
        PathOp::MoveTo(3.0, 11.0),
        PathOp::VerticalTo(10.0),
        PathOp::ArcTo {
            rx: 4.0,
            ry: 4.0,
            rotation: 0.0,
            large: false,
            sweep: true,
            to: (7.0, 6.0),
        },
        PathOp::HorizontalTo(21.0),
    ]),
    Shape::Path(&[
        PathOp::MoveTo(7.0, 22.0),
        PathOp::LineTo(3.0, 18.0),
        PathOp::LineTo(7.0, 14.0),
    ]),
    Shape::Path(&[
        PathOp::MoveTo(21.0, 13.0),
        PathOp::VerticalTo(14.0),
        PathOp::ArcTo {
            rx: 4.0,
            ry: 4.0,
            rotation: 0.0,
            large: false,
            sweep: true,
            to: (17.0, 18.0),
        },
        PathOp::HorizontalTo(3.0),
    ]),
];

/// `image`
static IMAGE: &[Shape] = &[
    Shape::RoundedRect {
        x: 3.0,
        y: 3.0,
        width: 18.0,
        height: 18.0,
        radius: 2.0,
    },
    Shape::Ellipse {
        cx: 9.0,
        cy: 9.0,
        r: 2.0,
    },
    Shape::Path(&[
        PathOp::MoveTo(21.0, 15.0),
        PathOp::LineTo(17.914, 11.914),
        PathOp::ArcTo {
            rx: 2.0,
            ry: 2.0,
            rotation: 0.0,
            large: false,
            sweep: false,
            to: (15.086, 11.914),
        },
        PathOp::LineTo(6.0, 21.0),
    ]),
];

/// `film`
static FILM: &[Shape] = &[
    Shape::RoundedRect {
        x: 3.0,
        y: 3.0,
        width: 18.0,
        height: 18.0,
        radius: 2.0,
    },
    Shape::Path(&[PathOp::MoveTo(7.0, 3.0), PathOp::VerticalTo(21.0)]),
    Shape::Path(&[PathOp::MoveTo(3.0, 7.5), PathOp::HorizontalTo(7.0)]),
    Shape::Path(&[PathOp::MoveTo(3.0, 12.0), PathOp::HorizontalTo(21.0)]),
    Shape::Path(&[PathOp::MoveTo(3.0, 16.5), PathOp::HorizontalTo(7.0)]),
    Shape::Path(&[PathOp::MoveTo(17.0, 3.0), PathOp::VerticalTo(21.0)]),
    Shape::Path(&[PathOp::MoveTo(17.0, 7.5), PathOp::HorizontalTo(21.0)]),
    Shape::Path(&[PathOp::MoveTo(17.0, 16.5), PathOp::HorizontalTo(21.0)]),
];

/// `music`
static MUSIC: &[Shape] = &[
    Shape::Path(&[
        PathOp::MoveTo(9.0, 18.0),
        PathOp::VerticalTo(5.0),
        PathOp::LineTo(21.0, 3.0),
        PathOp::VerticalTo(16.0),
    ]),
    Shape::Ellipse {
        cx: 6.0,
        cy: 18.0,
        r: 3.0,
    },
    Shape::Ellipse {
        cx: 18.0,
        cy: 16.0,
        r: 3.0,
    },
];

/// `file`
static FILE: &[Shape] = &[
    Shape::Path(&[
        PathOp::MoveTo(6.0, 22.0),
        PathOp::ArcTo {
            rx: 2.0,
            ry: 2.0,
            rotation: 0.0,
            large: false,
            sweep: true,
            to: (4.0, 20.0),
        },
        PathOp::VerticalTo(4.0),
        PathOp::ArcTo {
            rx: 2.0,
            ry: 2.0,
            rotation: 0.0,
            large: false,
            sweep: true,
            to: (6.0, 2.0),
        },
        PathOp::HorizontalTo(14.0),
        PathOp::ArcTo {
            rx: 2.4,
            ry: 2.4,
            rotation: 0.0,
            large: false,
            sweep: true,
            to: (15.704, 2.706),
        },
        PathOp::LineTo(19.292, 6.294),
        PathOp::ArcTo {
            rx: 2.4,
            ry: 2.4,
            rotation: 0.0,
            large: false,
            sweep: true,
            to: (20.0, 8.0),
        },
        PathOp::VerticalTo(20.0),
        PathOp::ArcTo {
            rx: 2.0,
            ry: 2.0,
            rotation: 0.0,
            large: false,
            sweep: true,
            to: (18.0, 22.0),
        },
        PathOp::Close,
    ]),
    Shape::Path(&[
        PathOp::MoveTo(14.0, 2.0),
        PathOp::VerticalTo(7.0),
        PathOp::ArcTo {
            rx: 1.0,
            ry: 1.0,
            rotation: 0.0,
            large: false,
            sweep: false,
            to: (15.0, 8.0),
        },
        PathOp::HorizontalTo(20.0),
    ]),
];

/// `file-text`
static FILE_TEXT: &[Shape] = &[
    Shape::Path(&[
        PathOp::MoveTo(6.0, 22.0),
        PathOp::ArcTo {
            rx: 2.0,
            ry: 2.0,
            rotation: 0.0,
            large: false,
            sweep: true,
            to: (4.0, 20.0),
        },
        PathOp::VerticalTo(4.0),
        PathOp::ArcTo {
            rx: 2.0,
            ry: 2.0,
            rotation: 0.0,
            large: false,
            sweep: true,
            to: (6.0, 2.0),
        },
        PathOp::HorizontalTo(14.0),
        PathOp::ArcTo {
            rx: 2.4,
            ry: 2.4,
            rotation: 0.0,
            large: false,
            sweep: true,
            to: (15.704, 2.706),
        },
        PathOp::LineTo(19.292, 6.294),
        PathOp::ArcTo {
            rx: 2.4,
            ry: 2.4,
            rotation: 0.0,
            large: false,
            sweep: true,
            to: (20.0, 8.0),
        },
        PathOp::VerticalTo(20.0),
        PathOp::ArcTo {
            rx: 2.0,
            ry: 2.0,
            rotation: 0.0,
            large: false,
            sweep: true,
            to: (18.0, 22.0),
        },
        PathOp::Close,
    ]),
    Shape::Path(&[
        PathOp::MoveTo(14.0, 2.0),
        PathOp::VerticalTo(7.0),
        PathOp::ArcTo {
            rx: 1.0,
            ry: 1.0,
            rotation: 0.0,
            large: false,
            sweep: false,
            to: (15.0, 8.0),
        },
        PathOp::HorizontalTo(20.0),
    ]),
    Shape::Path(&[PathOp::MoveTo(10.0, 9.0), PathOp::HorizontalTo(8.0)]),
    Shape::Path(&[PathOp::MoveTo(16.0, 13.0), PathOp::HorizontalTo(8.0)]),
    Shape::Path(&[PathOp::MoveTo(16.0, 17.0), PathOp::HorizontalTo(8.0)]),
];

/// `folder`
static FOLDER: &[Shape] = &[Shape::Path(&[
    PathOp::MoveTo(20.0, 20.0),
    PathOp::ArcTo {
        rx: 2.0,
        ry: 2.0,
        rotation: 0.0,
        large: false,
        sweep: false,
        to: (22.0, 18.0),
    },
    PathOp::VerticalTo(8.0),
    PathOp::ArcTo {
        rx: 2.0,
        ry: 2.0,
        rotation: 0.0,
        large: false,
        sweep: false,
        to: (20.0, 6.0),
    },
    PathOp::HorizontalTo(12.1),
    PathOp::ArcTo {
        rx: 2.0,
        ry: 2.0,
        rotation: 0.0,
        large: false,
        sweep: true,
        to: (10.41, 5.1),
    },
    PathOp::LineTo(9.6, 3.9),
    PathOp::ArcTo {
        rx: 2.0,
        ry: 2.0,
        rotation: 0.0,
        large: false,
        sweep: false,
        to: (7.93, 3.0),
    },
    PathOp::HorizontalTo(4.0),
    PathOp::ArcTo {
        rx: 2.0,
        ry: 2.0,
        rotation: 0.0,
        large: false,
        sweep: false,
        to: (2.0, 5.0),
    },
    PathOp::VerticalTo(18.0),
    PathOp::ArcTo {
        rx: 2.0,
        ry: 2.0,
        rotation: 0.0,
        large: false,
        sweep: false,
        to: (4.0, 20.0),
    },
    PathOp::Close,
])];

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use zest_core::{menu_for_selection, Selection};

    const ALL_ICONS: &[Icon] = &[
        Icon::Archive,
        Icon::PackageOpen,
        Icon::Repeat,
        Icon::Image,
        Icon::Film,
        Icon::Music,
        Icon::FileText,
        Icon::Folder,
        Icon::File,
    ];

    /// The point halfway along a cubic Bézier, p(0.5)."
    fn cubic_midpoint(curve: &[(f32, f32); 3], from: (f32, f32)) -> (f32, f32) {
        let [c1, c2, end] = *curve;
        let blend = |p0: f32, p1: f32, p2: f32, p3: f32| (p0 + 3.0 * p1 + 3.0 * p2 + p3) / 8.0;
        (
            blend(from.0, c1.0, c2.0, end.0),
            blend(from.1, c1.1, c2.1, end.1),
        )
    }

    fn sel(names: &[&str]) -> Selection {
        Selection::new(names.iter().map(PathBuf::from).collect())
    }

    fn leaf(label: &str, action: MenuAction) -> MenuNode {
        MenuNode {
            label: label.to_string(),
            action: Some(action),
            category: None,
            children: Vec::new(),
        }
    }

    /// The properties that matter to the renderer, stated so they do not depend
    /// on which of the two possible centres the endpoints resolve to: the run is
    /// continuous, it lands exactly on the arc's end, and it bulges to the right
    /// radius off the chord.
    #[test]
    fn a_semicircle_is_two_cubics_at_the_right_radius() {
        // A chord of exactly 2r is a semicircle, so the centre is the chord's
        // midpoint and the curve's midpoint is one radius perpendicular to it.
        let cubics = arc_cubics((5.0, 0.0), (15.0, 0.0), 5.0, 5.0, 0.0, false, true);
        assert_eq!(cubics.len(), 2, "half a turn is two quarter cubics");
        let second = &cubics[1];
        assert!(
            (second[2].0 - 15.0).abs() < 1e-4 && second[2].1.abs() < 1e-4,
            "the run should land on the arc's end, read {:?}",
            second[2]
        );
        // Radius: both quarters' midpoints sit on a radius-5 circle about (10, 0).
        // Walking them checks the run stays on the arc the whole way, not just at
        // its ends.
        let start = (5.0, 0.0);
        let mut pen = start;
        for (index, cubic) in cubics.iter().enumerate() {
            let mid = cubic_midpoint(cubic, pen);
            let radius = (mid.0 - 10.0).hypot(mid.1);
            assert!(
                (radius - 5.0).abs() < 0.01,
                "quarter {index} left the radius-5 circle: {mid:?} at r={radius}"
            );
            pen = cubic[2];
        }
    }

    #[test]
    fn a_quarter_arc_is_one_cubic_that_starts_on_the_chord() {
        // From (10, 0) to (0, 10) at radius 10: a quarter turn.
        let cubics = arc_cubics((10.0, 0.0), (0.0, 10.0), 10.0, 10.0, 0.0, false, true);
        assert_eq!(cubics.len(), 1, "a quarter turn fits one cubic");
        let curve = &cubics[0];
        assert!(
            (curve[2].0 - 0.0).abs() < 1e-3 && (curve[2].1 - 10.0).abs() < 1e-3,
            "the run should land on the arc's end, read {:?}",
            curve[2]
        );
        // The control points sit inside the arc's own quarter, not outside it, so
        // the curve stays within the icon rather than bulging past it.
        for (index, control) in curve[..2].iter().enumerate() {
            let corner_distance = control.0.hypot(control.1);
            assert!(
                corner_distance < 15.0,
                "control {index} at {control:?} is outside the quarter's bounding box"
            );
        }
    }

    /// Sweep direction decides which way round the arc bulges. Reversing it must
    /// put the curve on the other side of the chord.
    #[test]
    fn the_sweep_flag_chooses_which_side_of_the_chord_the_arc_bulges_to() {
        let one_way = arc_cubics((5.0, 0.0), (15.0, 0.0), 5.0, 5.0, 0.0, false, true);
        let other = arc_cubics((5.0, 0.0), (15.0, 0.0), 5.0, 5.0, 0.0, false, false);
        let above = cubic_midpoint(&one_way[0], (5.0, 0.0));
        let below = cubic_midpoint(&other[0], (5.0, 0.0));
        assert!(
            above.1 * below.1 < 0.0,
            "the two sweeps should bulge to opposite sides, read {above:?} and {below:?}"
        );
        assert!((above.1.abs() - below.1.abs()).abs() < 0.01);
    }

    #[test]
    fn a_degenerate_arc_becomes_a_straight_line() {
        let cubics = arc_cubics((0.0, 0.0), (3.0, 0.0), 0.0, 0.0, 0.0, false, false);
        assert_eq!(cubics.len(), 1);
        assert_eq!(cubics[0][2], (3.0, 0.0));
    }

    #[test]
    fn an_arc_to_its_own_start_draws_nothing() {
        assert!(arc_cubics((2.0, 2.0), (2.0, 2.0), 2.0, 2.0, 0.0, false, true).is_empty());
    }

    /// Radii too small for the endpoints get scaled up until they span them. Two
    /// points 10 apart on a circle of radius 1 only work if the radius grows, and
    /// this is what keeps a rounded corner from collapsing.
    #[test]
    fn radii_too_small_for_the_endpoints_get_scaled_up() {
        let cubics = arc_cubics((0.0, 0.0), (10.0, 0.0), 1.0, 1.0, 0.0, false, false);
        // Scaled until it spans them, the radius is 5 and the chord 10, so the arc
        // between them is half a turn.
        assert_eq!(cubics.len(), 2);
        assert!((cubics[1][2].0 - 10.0).abs() < 1e-4 && cubics[1][2].1.abs() < 1e-4);
        // And it bulges to the scaled radius about the scaled centre, not to the
        // one that was asked for. The first quarter's midpoint sits at 45° on a
        // radius-5 circle about (5, 0).
        let mid = cubic_midpoint(&cubics[0], (0.0, 0.0));
        let radius = (mid.0 - 5.0).hypot(mid.1);
        assert!(
            (radius - 5.0).abs() < 0.01,
            "the scaled arc should reach 5 off its centre, read {mid:?} at r={radius}"
        );
    }

    /// The bounding box of an icon's outline, in viewbox coordinates.
    ///
    /// Paths are walked with the same pen the renderer uses, so a `HorizontalTo` or
    /// `VerticalTo` contributes its real coordinate rather than a guess. That makes
    /// this a check on the transcription as much as on the data.
    pub fn bounds(icon: Icon) -> (f32, f32, f32, f32) {
        let mut box_ = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        let mut see = |x: f32, y: f32| {
            box_.0 = box_.0.min(x);
            box_.1 = box_.1.min(y);
            box_.2 = box_.2.max(x);
            box_.3 = box_.3.max(y);
        };
        for shape in icon.shapes() {
            match shape {
                Shape::RoundedRect {
                    x,
                    y,
                    width,
                    height,
                    ..
                } => {
                    see(*x, *y);
                    see(x + width, y + height);
                }
                Shape::Ellipse { cx, cy, r } => {
                    see(cx - r, cy - r);
                    see(cx + r, cy + r);
                }
                Shape::Path(ops) => {
                    let mut pen = (0.0_f32, 0.0_f32);
                    for op in *ops {
                        match op {
                            PathOp::MoveTo(x, y) | PathOp::LineTo(x, y) => {
                                pen = (*x, *y);
                                see(pen.0, pen.1);
                            }
                            PathOp::HorizontalTo(x) => {
                                pen = (*x, pen.1);
                                see(pen.0, pen.1);
                            }
                            PathOp::VerticalTo(y) => {
                                pen = (pen.0, *y);
                                see(pen.0, pen.1);
                            }
                            PathOp::ArcTo { to, .. } => {
                                // The curve between the endpoints of an arc stays within
                                // the box its endpoints and controls define, and for the
                                // convex corners Lucide uses that is the chord's box.
                                pen = *to;
                                see(pen.0, pen.1);
                            }
                            PathOp::QuadTo(x1, y1, x, y) => {
                                pen = (*x, *y);
                                see(*x1, *y1);
                                see(pen.0, pen.1);
                            }
                            PathOp::CubicTo(x1, y1, x2, y2, x, y) => {
                                pen = (*x, *y);
                                see(*x1, *y1);
                                see(*x2, *y2);
                                see(pen.0, pen.1);
                            }
                            PathOp::Close => {}
                        }
                    }
                }
            }
        }
        box_
    }

    #[test]
    fn convert_targets_get_the_icon_of_their_kind() {
        for (ext, icon) in [
            ("png", Icon::Image),
            ("webp", Icon::Image),
            ("mp4", Icon::Film),
            ("flac", Icon::Music),
            ("json", Icon::FileText),
        ] {
            let node = leaf(ext, MenuAction::Convert { ext: ext.into() });
            assert_eq!(icon_for(&node), icon, "{ext} should carry {icon:?}");
        }
    }

    #[test]
    fn a_target_of_an_unknown_kind_still_gets_an_icon() {
        let node = leaf("xyz", MenuAction::Convert { ext: "xyz".into() });
        assert_eq!(icon_for(&node), Icon::File);
    }

    #[test]
    fn archive_and_extract_leaves_get_their_own_pictures() {
        assert_eq!(
            icon_for(&leaf("zip", MenuAction::Archive { ext: "zip".into() })),
            Icon::Archive
        );
        assert_eq!(icon_for(&leaf("Here", MenuAction::Extract)), Icon::Folder);
    }

    #[test]
    fn ring_one_categories_get_their_own_pictures() {
        let menu = menu_for_selection(&sel(&["a.png"]));
        let icons: Vec<Icon> = menu.iter().map(icon_for).collect();
        assert_eq!(icons, vec![Icon::Repeat, Icon::Archive]);

        let menu = menu_for_selection(&sel(&["a.zip"]));
        assert_eq!(
            menu.iter().map(icon_for).collect::<Vec<_>>(),
            vec![Icon::PackageOpen]
        );
    }

    #[test]
    fn a_node_with_neither_action_nor_category_still_paints_something() {
        let node = MenuNode {
            label: "mystery".into(),
            action: None,
            category: None,
            children: Vec::new(),
        };
        assert_eq!(icon_for(&node), Icon::File);
    }

    /// Every icon has to stay inside the viewbox, or it gets clipped by the
    /// wedge it sits in.
    #[test]
    fn every_icon_stays_inside_the_viewbox() {
        for icon in ALL_ICONS.iter().copied() {
            assert!(!icon.shapes().is_empty(), "{icon:?} has no shapes");
            let (min_x, min_y, max_x, max_y) = bounds(icon);
            assert!(
                (-0.01..=VIEWBOX + 0.01).contains(&min_x)
                    && (-0.01..=VIEWBOX + 0.01).contains(&min_y)
                    && (-0.01..=VIEWBOX + 0.01).contains(&max_x)
                    && (-0.01..=VIEWBOX + 0.01).contains(&max_y),
                "{icon:?} reaches outside the 24×24 viewbox: ({min_x}, {min_y}) to ({max_x}, {max_y})"
            );
        }
    }

    /// The renderer draws these unscaled-in, with no stroke inset of its own, so
    /// half the stroke has to land inside the viewbox on Lucide's account. Every
    /// icon carries at least a full unit of margin, and this pins that: without
    /// it, an icon authored against the grid could have half its outline cut off.
    #[test]
    fn every_icon_leaves_room_for_its_own_stroke() {
        let margin = STROKE_WIDTH / 2.0;
        for icon in ALL_ICONS.iter().copied() {
            let (min_x, min_y, max_x, max_y) = bounds(icon);
            assert!(
                min_x >= margin - 0.01 && min_y >= margin - 0.01,
                "{icon:?} starts at ({min_x}, {min_y}), inside half a stroke of the edge"
            );
            assert!(
                max_x <= VIEWBOX - margin + 0.01 && max_y <= VIEWBOX - margin + 0.01,
                "{icon:?} ends at ({max_x}, {max_y}), inside half a stroke of the edge"
            );
        }
    }
}

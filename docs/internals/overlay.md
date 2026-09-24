# Overlay rendering

The radial menu is custom-drawn vector graphics on a transparent layered
window: arcs, filled sectors, gradients, anti-aliased text. It is not a UI
framework menu. That is the only way to get a floating translucent radial
overlay with the right compositing. WinPie (open-source Rust radial menu for
Windows) is the architectural reference.

## Feel contract

- Dark semi-transparent surface, frosted acrylic/mica blur when available.
- Active sector: smoothed 1–3 color gradient (user-configurable).
- Inactive sectors: muted dark gray with thin borders.
- Sector transitions: quick ease-out around 200ms (`SECTOR_TRANSITION_MS`).
- Center: file thumbnail, or file-count badge for multi-select.
- Type: Segoe UI Variable default (configurable). Icons: Lucide as starting point.

## Performance trick

The window is pre-created and hidden (`Overlay::precreate`). A dedicated Win32
message-loop thread owns the HWND, Direct2D render target, and premultiplied
BGRA DIB. Activation positions the existing layered surface at the cursor and
uses `UpdateLayeredWindow`; it never builds the surface from scratch.

## Testable core

Sector geometry and angle hit-testing (`layout_sectors`, `hit_test`,
`ring_labels`) are pure logic with unit tests. The Direct2D renderer consumes
that model and owns only pixels, blur, and animation. Native precreate,
activation, and teardown are covered by the overlay crate's Windows test.

## DPI and edges

DPI awareness and cursor-near-screen-edge placement land in the polish
milestone. Keep geometry in one place so those fixes do not leak into menu
logic.

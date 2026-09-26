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

Sector geometry and point hit-testing (`layout_sectors`, `hit_test_at_point`)
are pure logic with unit tests. `RingModel` owns the menu: it keeps the
`MenuNode` tree, expands a category into its children, and resolves a leaf
click to that node's `MenuAction`. The Direct2D renderer consumes that model
and owns only pixels, blur, and animation. Native precreate, activation, and
the leaf-click-to-channel path are covered by the overlay crate's Windows
tests.

## Choice channel

`Overlay::precreate()` returns the overlay plus a `MenuChoices` receiver. The
overlay thread owns the HWND and sends the picked `MenuAction` over a tokio
channel, so the app never reaches into the window. `zest-app` dispatches it
against the selection the visible menu was built from: the overlay is
`WS_EX_NOACTIVATE`, so Explorer keeps focus and its selection cannot change
while the menu is up.

## DPI and edges

DPI awareness and cursor-near-screen-edge placement land in the polish
milestone. Keep geometry in one place so those fixes do not leak into menu
logic.

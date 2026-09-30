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

## Hover animation

`SectorEmphasis` is the testable half of the hover: a lit amount per sector,
eased from 0 to 1 over `SECTOR_TRANSITION_MS` and re-aimable mid-fade without
jumping. The renderer owns the frame timer: a hover change starts a `WM_TIMER`
at `ANIMATION_FRAME_MS`, each tick advances the fades and repaints, and the
timer is killed once every fade settles — so an idle ring costs nothing.

The active sector is the muted fill with the configured gradient painted over
it at `emphasis` opacity, which cross-fades in both directions in one blend.
The gradient comes from the `Gradient` in settings: stops are parsed to
straight-alpha colors and spread evenly across the sector's own radius, from
the inner edge to the outer edge. A gradient that could not be built falls back
to the flat mean color, so a gradient failure costs the ramp and not the hover.

Settings are re-read on every show, so a gradient picked in the settings window
shows up on the next hotkey without a restart.

## Why the gradient is painted as bands

Not because the render target cannot do it. `ID2D1DCRenderTarget` was the
first suspect and it is exonerated: a bare black-to-white ramp fills a rectangle
with one flat colour, at 0.737 of full brightness, on a DC render target *and*
on a WIC-backed `ID2D1BitmapRenderTarget`, with the axis set both in the brush
properties and again with `SetStartPoint`/`SetEndPoint`. A solid brush on
either target reads back exactly, so the readback and the target are both fine.
The flat colour tracks the stop list — red to green paints a flat yellow — so
the brush has the stops and evaluates its axis at a single point.

The `windows` crate is the reason. Three separate defects, in 0.61 and in
0.62.2 alike:

- `D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES` is declared as the two points alone,
  16 bytes, where the platform reads 28. The trailing interpolation, extend,
  and alpha modes come off the stack.
- `ID2D1LinearGradientBrush`'s method table omits `ID2D1GradientBrush`, so its
  four methods sit one slot early. `GetStartPoint` takes the process down.
- `CreateLinearGradientBrush`'s binding drops the `riid` parameter, shifting
  every argument after the stop collection by one.

So the ramp is painted as `RAMP_BANDS` concentric solid wedges filled with
`ramp_color` at each band's midpoint, overlapping by `BAND_SEAM` so rounding
cannot leave hairlines. They are a workaround for the bindings, not for the
target. `the_windows_binding_under_reads_linear_gradient_properties` pins the
16-versus-28 gap and fails when the binding is fixed, which is the signal to
delete the bands. Bumping `windows` is not that signal: 0.62.2 has all three
defects. Building the brush through a hand-written vtable entry is the other
option, and it is a real cost — the 28-byte struct and the `riid` have to be
declared, and the `ID2D1LinearGradientBrush` methods cannot be called at all.

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

# First run: select, hotkey, radial menu

Zest has no document window. The whole interaction is: select files, press the
hotkey, flick toward the action you want.

## The interaction

1. Select one or more files in Explorer or on the Desktop.
2. Press your hotkey (`Shift+F` by default). Zest reads the Explorer selection
   through COM, classifies the files, and draws the radial menu at your cursor.
3. Ring 1 shows categories: **Convert**, **Archive**, or **Extract**, filtered
   to what makes sense for the selection.
4. Picking a category replaces the ring with its targets (ring 2). A second
   hotkey jumps straight to the Convert ring.
5. Click a target. The menu vanishes and the job runs in the background. A
   toast confirms completion.

## What ring 1 shows

| Selection | Ring 1 |
| --- | --- |
| Single PNG (or other convertible file) | Convert, Archive |
| Single `.zip` / `.tar` / `.tar.gz` / `.gzip` | Extract |
| Mixed file types, or folders | Archive only |
| Virtual folders (Recycle Bin, This PC) | Nothing; the menu does not open |

Conversion requires files of the same type. Mixed selections can only be
archived.

## Why radial

Like Tangerine on Mac, the same action always sits at the same angle. After a
week you stop reading labels and flick toward the direction you know.

## Center readout

The ring center shows a thumbnail of the selected file, or a file-count badge
when several are selected, so you always have confirmation of what you are
acting on.

## Next steps

- [Converting files](./converting.md): formats per kind and quality notes.
- [Archives](./archives.md): creating and extracting.
- [Hotkey](./hotkey.md): changing the shortcut.

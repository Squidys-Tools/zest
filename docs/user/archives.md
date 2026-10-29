# Archives: create and extract

Archives are the fallback action: any selection of files or folders can be
archived, and supported archives can be extracted.

## Create

Select files and/or folders, open the menu, pick **Archive**, then choose:

| Target | Notes |
| --- | --- |
| `.zip` | Best for sharing with anyone |
| `.tar` | Uncompressed bundle |
| `.tar.gz` | Compressed bundle |
| `.gz` | Single-file compression: `photo.png` becomes `photo.png.gz` |

The whole selection becomes one archive: a folder packs with its contents
under its own name, a file under its file name. A single item names the
archive after itself; several items name it after the folder they sit in. `.gz`
compresses one file only, so picking it for a folder or several files stops
with a message to choose another format.
Archives land beside the original (or in your configured output folder) and
never overwrite an existing file — a `name (1)` suffix is added instead.

## Extract

Select a `.zip`, `.tar`, `.tar.gz`, `.tgz`, or `.gz` file and the menu shows
**Extract** only. Contents land next to the archive: zip and tar bundles
unpack into a folder named after the archive, while a `.gz` decompresses
back to the single file it was made from. Entries that would escape the
target folder (`..` paths) are refused rather than extracted.

## Next steps

- [First run](./first-run.md): when Archive is the only option.
- [Settings](./settings.md): output location for created archives.

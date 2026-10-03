//! Two-ring radial menu model (PRD §How It Works).
//!
//! Ring 1: categories filtered by selection.
//! Ring 2: leaf targets after picking a category (or via second hotkey for Convert).

use crate::file_kind::{classify_path, FileKind};
use crate::Selection;

/// Ring-1 categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionCategory {
    Convert,
    Archive,
    Extract,
}

/// Ring-1 filtering rules:
/// - single archive (.zip/.tar/.tgz/.gz) → Extract only
/// - uniform non-archive kind → Convert + Archive
/// - folders or mixed kinds → Archive only (conversion needs uniform type)
/// - empty → none
pub fn categories_for_selection(sel: &Selection) -> Vec<ActionCategory> {
    if sel.is_empty() {
        return vec![];
    }
    let kinds: Vec<FileKind> = sel.files.iter().map(|p| classify_path(p)).collect();

    // Single extractable archive → Extract only.
    if kinds.len() == 1 {
        if let Some(p) = sel.files.first() {
            let ext = p
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            if kinds[0] == FileKind::Archive && FileKind::is_extractable_archive(&ext) {
                return vec![ActionCategory::Extract];
            }
        }
    }

    let first = kinds[0];
    let uniform = kinds.iter().all(|k| *k == first);
    match (uniform, first) {
        // Archives selected directly are archivable, not convertible.
        (true, FileKind::Archive) => vec![ActionCategory::Extract, ActionCategory::Archive],
        (true, FileKind::Other) => vec![ActionCategory::Archive],
        (true, _) => vec![ActionCategory::Convert, ActionCategory::Archive],
        (false, _) => vec![ActionCategory::Archive],
    }
}

/// Ring-2 Convert leaves per kind (output extensions, no dot).
///
/// Only targets the engine can actually produce appear here. `heic` is absent
/// on purpose: nothing decodes HEVC yet (SQU-59), and offering a target that
/// always fails is worse than not offering it. `pdf` is likewise absent because
/// it belongs to the text engine (SQU-51), not the image one.
pub fn convert_targets(kind: FileKind) -> &'static [&'static str] {
    match kind {
        FileKind::Image => &["png", "jpg", "webp", "bmp", "gif", "tiff", "ico"],
        FileKind::Video => &["mp4", "webm", "avi", "mkv", "mov", "gif"],
        FileKind::Audio => &["mp3", "wav", "flac", "aac", "ogg", "m4a", "opus"],
        FileKind::TextData => &["txt", "csv", "json", "xml", "yaml", "toml", "md", "pdf"],
        _ => &[],
    }
}

/// Ring-2 Archive leaves (PRD: zip, tar, tar.gz, gzip).
pub fn archive_targets() -> &'static [&'static str] {
    &["zip", "tar", "tar.gz", "gzip"]
}

/// What a picked leaf does. The overlay only reports this; dispatching it is
/// the app's job, so every string the menu shows lives here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuAction {
    /// Convert the selection to `ext` (no leading dot).
    Convert { ext: String },
    /// Create an archive of the selection in `ext` format.
    Archive { ext: String },
    /// Extract the selected archive next to itself.
    Extract,
}

/// One sector in the radial menu. Ring-1 categories carry no action and only
/// fan out; ring-2 leaves carry exactly one action. `category` is what tells the
/// two apart without matching on the label, which is how the overlay picks an
/// icon for a category that has no action yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuNode {
    pub label: String,
    pub action: Option<MenuAction>,
    pub category: Option<ActionCategory>,
    pub children: Vec<MenuNode>,
}

impl MenuNode {
    /// Ring-1 category: it fans out, so it has no action of its own.
    fn category(category: ActionCategory, label: &str, children: Vec<MenuNode>) -> Self {
        debug_assert!(children.iter().all(|child| child.action.is_some()));
        Self {
            label: label.to_string(),
            action: None,
            category: Some(category),
            children,
        }
    }

    /// Ring-2 leaf: picking it runs this action.
    fn leaf(label: &str, action: MenuAction) -> Self {
        Self {
            label: label.to_string(),
            action: Some(action),
            category: None,
            children: Vec::new(),
        }
    }
}

fn leaves(targets: &[&str], action: fn(&str) -> MenuAction) -> Vec<MenuNode> {
    targets
        .iter()
        .map(|target| MenuNode::leaf(target, action(target)))
        .collect()
}

fn convert_ext(ext: &str) -> MenuAction {
    MenuAction::Convert {
        ext: ext.to_string(),
    }
}

fn archive_ext(ext: &str) -> MenuAction {
    MenuAction::Archive {
        ext: ext.to_string(),
    }
}

/// The two-ring menu for a selection: ring 1 categories, ring 2 leaf targets.
/// Empty when the selection is empty.
pub fn menu_for_selection(sel: &Selection) -> Vec<MenuNode> {
    let Some(first) = sel.files.first() else {
        return vec![];
    };
    let kind = classify_path(first);
    categories_for_selection(sel)
        .into_iter()
        .map(|category| match category {
            ActionCategory::Convert => MenuNode::category(
                category,
                "Convert",
                leaves(convert_targets(kind), convert_ext),
            ),
            ActionCategory::Archive => {
                MenuNode::category(category, "Archive", leaves(archive_targets(), archive_ext))
            }
            ActionCategory::Extract => MenuNode::category(
                category,
                "Extract",
                vec![MenuNode::leaf("Here", MenuAction::Extract)],
            ),
        })
        .collect()
}

/// Ring 2 of the Convert category on its own, for the Convert hotkey path.
pub fn convert_menu_for_selection(sel: &Selection) -> Vec<MenuNode> {
    match sel.files.first() {
        Some(first) => leaves(convert_targets(classify_path(first)), convert_ext),
        None => vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn sel(names: &[&str]) -> Selection {
        Selection::new(names.iter().map(PathBuf::from).collect())
    }

    /// A target the engine always rejects must not reach the ring: offering a
    /// leaf that cannot work is worse than not offering it. HEVC decoding is
    /// SQU-59, PDF output is the text engine's job (SQU-51).
    #[test]
    fn the_convert_ring_offers_no_dead_targets() {
        let targets = convert_targets(FileKind::Image);
        for dead in ["heic", "pdf", "svg"] {
            assert!(
                !targets.contains(&dead),
                "{dead} cannot be produced yet but is offered in ring 2"
            );
        }
    }

    #[test]
    fn the_convert_ring_still_offers_the_real_raster_targets() {
        let targets = convert_targets(FileKind::Image);
        for live in ["png", "jpg", "webp", "bmp", "gif", "tiff", "ico"] {
            assert!(targets.contains(&live), "{live} should be offered");
        }
    }

    #[test]
    fn png_gets_convert_and_archive() {
        assert_eq!(
            categories_for_selection(&sel(&["photo.png"])),
            vec![ActionCategory::Convert, ActionCategory::Archive]
        );
    }

    #[test]
    fn zip_gets_extract_only() {
        assert_eq!(
            categories_for_selection(&sel(&["a.zip"])),
            vec![ActionCategory::Extract]
        );
    }

    #[test]
    fn mixed_gets_archive_only() {
        assert_eq!(
            categories_for_selection(&sel(&["a.png", "b.mp3"])),
            vec![ActionCategory::Archive]
        );
    }

    #[test]
    fn convert_category_fans_out_to_targets() {
        let menu = menu_for_selection(&sel(&["photo.png"]));
        let convert = &menu[0];
        assert_eq!(convert.label, "Convert");
        assert_eq!(convert.action, None);
        assert_eq!(
            convert.children.first().map(|c| c.action.clone()),
            Some(Some(MenuAction::Convert {
                ext: "png".to_string()
            }))
        );
        assert!(convert.children.iter().all(|c| c.children.is_empty()));
    }

    #[test]
    fn archive_category_fans_out_to_archive_formats() {
        let menu = menu_for_selection(&sel(&["a.png", "b.mp3"]));
        assert_eq!(menu.len(), 1);
        let archive = &menu[0];
        assert_eq!(archive.label, "Archive");
        assert_eq!(
            archive.children.last().and_then(|c| c.action.clone()),
            Some(MenuAction::Archive {
                ext: "gzip".to_string()
            })
        );
    }

    #[test]
    fn extract_category_offers_here() {
        let menu = menu_for_selection(&sel(&["bundle.zip"]));
        assert_eq!(menu.len(), 1);
        assert_eq!(menu[0].label, "Extract");
        assert_eq!(menu[0].children.len(), 1);
        assert_eq!(menu[0].children[0].action, Some(MenuAction::Extract));
    }

    #[test]
    fn empty_selection_has_no_menu() {
        assert!(menu_for_selection(&Selection::default()).is_empty());
        assert!(convert_menu_for_selection(&Selection::default()).is_empty());
    }

    #[test]
    fn convert_menu_is_the_bare_ring_two() {
        let menu = convert_menu_for_selection(&sel(&["photo.png"]));
        assert!(menu.iter().all(|node| node.action.is_some()));
        assert_eq!(
            menu.first().and_then(|node| node.action.clone()),
            Some(MenuAction::Convert {
                ext: "png".to_string()
            })
        );
    }

    /// The overlay picks a category's icon from `category` rather than from its
    /// label, because a category has no action. If a node ever carried both, or
    /// a category lost its category, the ring would draw the wrong picture
    /// silently — so pin the two as mutually exclusive.
    #[test]
    fn a_node_is_either_a_category_or_a_leaf_never_both() {
        fn check(nodes: &[MenuNode], path: &str) {
            for node in nodes {
                match (node.category, node.action.is_some()) {
                    (Some(_), true) => panic!("{path}/{} is both", node.label),
                    (Some(_), false) => {
                        assert!(
                            !node.children.is_empty(),
                            "{path}/{} fans out to nothing",
                            node.label
                        )
                    }
                    (None, true) => assert!(
                        node.children.is_empty(),
                        "{path}/{} is a leaf with children",
                        node.label
                    ),
                    (None, false) => panic!("{path}/{} is neither", node.label),
                }
                check(&node.children, &format!("{path}/{}", node.label));
            }
        }
        for name in ["a.png", "a.zip", "a.mp3", "a.json"] {
            check(&menu_for_selection(&sel(&[name])), name);
            check(&convert_menu_for_selection(&sel(&[name])), name);
        }
    }
}

use deadlock::patch_kind::{PatchKind, classify};

#[test]
fn forum_posts_are_patches() {
    assert_eq!(classify("forum", "Update 14 October"), Some(PatchKind::Patch));
}

#[test]
fn steam_posts_titled_update_are_patches() {
    assert_eq!(
        classify("steam", "Halloween 2026 + Gameplay Update"),
        Some(PatchKind::Patch)
    );
    assert_eq!(
        classify("steam", " Minor Update - 11-04-2026"),
        Some(PatchKind::Patch)
    );
}

#[test]
fn other_steam_posts_are_news() {
    assert_eq!(classify("steam", "Ranked - Beta Season 2"), Some(PatchKind::News));
}

#[test]
fn unknown_sources_are_ignored() {
    assert_eq!(classify("reddit", "Update"), None);
}

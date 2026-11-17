#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchKind {
    Patch,
    News,
}

impl PatchKind {
    #[must_use]
    pub const fn from_news(news: bool) -> Self {
        if news { Self::News } else { Self::Patch }
    }

    #[must_use]
    pub const fn is_news(self) -> bool {
        matches!(self, Self::News)
    }
}

/// `None` for sources the announcer ignores.
#[must_use]
pub fn classify(source: &str, title: &str) -> Option<PatchKind> {
    match source {
        "forum" => Some(PatchKind::Patch),
        "steam" if title.to_lowercase().contains("update") => Some(PatchKind::Patch),
        "steam" => Some(PatchKind::News),
        _ => None,
    }
}

#[derive(Debug, PartialEq)]
pub struct Chars {
    pub ring: &'static str,
    pub tidied: &'static str,
    pub unchanged: &'static str,
    pub lint_free: &'static str,
    pub lint_dirty: &'static str,
    pub empty: &'static str,
    pub bullet: &'static str,
}

pub const FUN_CHARS: Chars = Chars {
    ring: "💍",
    tidied: "💧",
    unchanged: "✨",
    lint_free: "💯",
    lint_dirty: "💩",
    empty: "⚫",
    bullet: "▶",
};

pub const BORING_CHARS: Chars = Chars {
    ring: ":",
    tidied: "*",
    unchanged: "|",
    lint_free: "|",
    lint_dirty: "*",
    empty: "_",
    bullet: "*",
};

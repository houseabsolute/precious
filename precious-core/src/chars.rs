#[derive(Debug, Eq, PartialEq)]
pub struct Chars {
    pub ring: &'static str,
    pub tidied: &'static str,
    pub unchanged: &'static str,
    pub unknown: &'static str,
    pub lint_free: &'static str,
    pub lint_dirty: &'static str,
    pub empty: &'static str,
    pub bullet: &'static str,
    pub execution_error: &'static str,
}

pub const FUN_CHARS: Chars = Chars {
    ring: "💍",
    tidied: "💧",
    unchanged: "✨",
    // Person shrugging with medium skin tone - it'd be cool to randomize the
    // skin tone and gender on each run but then this wouldn't be static and
    // the chars wouldn't be constants and I'd have to turn this all into
    // functions.
    unknown: "🤷🏽",
    lint_free: "💯",
    lint_dirty: "💩",
    empty: "⚫",
    bullet: "▶",
    execution_error: "💥",
};

pub const BORING_CHARS: Chars = Chars {
    ring: ":",
    tidied: "*",
    unchanged: "|",
    unknown: "?",
    lint_free: "|",
    lint_dirty: "*",
    empty: "_",
    bullet: "*",
    execution_error: "!",
};

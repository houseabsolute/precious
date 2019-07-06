pub fn dirs() -> Vec<String> {
    [".git", ".hg", ".svn"]
        .iter()
        .map(|&s| String::from(s))
        .collect()
}

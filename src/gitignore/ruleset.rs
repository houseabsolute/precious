use failure::Error;
use globset::{Candidate, GlobBuilder, GlobSet, GlobSetBuilder};
use std::path::{Path, PathBuf};

/// Represents a set of rules that can be checked against to see if a path should be ignored within
/// a Git repository.
///
/// The performance characteristics of this are such that it is much better to try and make a single
/// instance of this to check as many paths against as possible - this is because the highest cost
/// is in constructing it, but checking against the compiled patterns is extremely cheap.
#[derive(Debug)]
pub struct RuleSet {
    root: PathBuf,
    pub(crate) rules: Vec<Rule>,
    tester: GlobSet,
}

impl RuleSet {
    /// Construct a ruleset, given a path that is the root of the repository, and a set of rules,
    /// which is a vector
    pub fn new<'a, P, I, S>(root: P, raw_rules: I) -> Result<RuleSet, Error>
    where
        P: AsRef<Path>,
        I: IntoIterator<Item = &'a S>,
        S: AsRef<str> + 'a,
    {
        // FIXME: Is there a better way without needing to hardcode a path here?
        let cleaned_root = Self::strip_prefix(root, Path::new("./"));

        let lines = raw_rules
            .into_iter()
            .map(RuleSet::parse_line)
            .collect::<Result<Vec<ParsedLine>, Error>>()?;

        let rules: Vec<Rule> = lines
            .iter()
            .filter_map(|parsed_line| {
                match parsed_line {
                    // FIXME: Remove this clone if possible, it's rank.
                    ParsedLine::WithRule(ref rule) => Some(rule.clone()),
                    _ => None,
                }
            })
            .collect();

        let mut tester_builder = GlobSetBuilder::new();

        // Add globs to globset.
        for rule in rules.iter() {
            let mut glob_builder = GlobBuilder::new(&rule.pattern);
            glob_builder.literal_separator(rule.anchored);
            let glob = glob_builder.build()?;
            tester_builder.add(glob);
        }

        let tester = tester_builder.build()?;

        Ok(RuleSet {
            root: cleaned_root,
            rules,
            tester,
        })
    }

    /// Check if the given path should be considered ignored as per the rules contained within
    /// the current ruleset.
    pub fn is_ignored<P: AsRef<Path>>(&self, path: P, is_dir: bool) -> bool {
        // FIXME: Is there a better way without needing to hardcode a path here?
        let mut cleaned_path = Self::strip_prefix(path.as_ref(), Path::new("./"));
        cleaned_path = Self::strip_prefix(cleaned_path.as_path(), &self.root);
        let candidate = Candidate::new(&cleaned_path);
        let results = self.tester.matches_candidate(&candidate);
        for idx in results.iter().rev() {
            let rule = &self.rules[*idx];

            // We must backtrack through the finds until we find one that is_dir
            // and rule.dir_only agree on.
            if rule.dir_only && !is_dir {
                continue;
            }

            return !rule.negation;
        }

        false
    }

    /// Given a raw pattern, parse it and attempt to construct a rule out of it. The pattern pattern
    /// rules are implemented as described in the documentation for Git at
    /// https://git-scm.com/docs/gitignore.
    fn parse_line<R: AsRef<str>>(raw_rule: R) -> Result<ParsedLine, Error> {
        // FIXME: Can we combine some of these string scans?
        let mut pattern = raw_rule.as_ref().trim();

        if pattern.is_empty() {
            return Ok(ParsedLine::Empty);
        }

        if pattern.starts_with('#') {
            return Ok(ParsedLine::Comment);
        }

        let negation = pattern.starts_with('!');
        if negation {
            pattern = pattern.trim_start_matches('!').trim();
        }

        let dir_only = pattern.ends_with('/');
        if dir_only {
            pattern = pattern.trim_end_matches('/').trim();
        }

        let absolute = pattern.starts_with('/');
        if absolute {
            pattern = pattern.trim_start_matches('/');
        }

        let anchored = absolute || pattern.contains('/');

        let mut cleaned_pattern = if !absolute && !pattern.starts_with("**/") {
            format!("**/{}", pattern.replace(r"\", ""))
        } else {
            pattern.replace(r"\", "")
        };

        // If the glob ends with `/**`, then we should only match everything
        // inside a directory, but not the directory itself. Standard globs
        // will match the directory. So we add `/*` to force the issue.
        if cleaned_pattern.ends_with("/**") {
            cleaned_pattern = format!("{}/*", cleaned_pattern);
        }

        Ok(ParsedLine::WithRule(Rule {
            pattern: cleaned_pattern, // FIXME: This is not zero-copy.
            anchored,
            dir_only,
            negation,
        }))
    }

    /// Given a path and a prefix, strip the prefix off the path. If the path does not begin with
    /// the given prefix, then return the path as is.
    fn strip_prefix<P: AsRef<Path>, PR: AsRef<Path>>(path: P, prefix: PR) -> PathBuf {
        path.as_ref()
            .strip_prefix(prefix.as_ref())
            .unwrap_or_else(|_| path.as_ref())
            .to_path_buf()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Rule {
    pub pattern: String,
    /// Whether this rule is anchored. If a rule is anchored (contains a slash)
    /// then wildcards inside the rule are not allowed to match a `/` in the
    /// pathname.
    pub anchored: bool,
    /// Whether this rule is only allowed to match directories.
    pub dir_only: bool,
    /// Whether the rule should, if it matches, negate any previously matching
    /// patterns. This flag has no effect if no previous patterns had matched.
    pub negation: bool,
}

#[derive(Debug)]
enum ParsedLine {
    Empty,
    Comment,
    WithRule(Rule),
}

#[cfg(test)]
mod test {
    use super::RuleSet;
    use std::path::Path;

    fn ruleset_from_rules<P: AsRef<Path>, S: AsRef<str>>(root: P, raw_rules: S) -> RuleSet {
        let rules: Vec<String> = raw_rules.as_ref().lines().map(|s| s.to_string()).collect();
        RuleSet::new(root, rules.iter()).unwrap()
    }

    macro_rules! ignored {
        ($name:ident, $root:expr, $rules:expr, $path:expr) => {
            ignored!($name, $root, $rules, $path, false, false);
        };
        ($name:ident, $root:expr, $rules:expr, $path:expr, $is_dir:expr) => {
            ignored!($name, $root, $rules, $path, $is_dir, false);
        };
        ($name:ident, $root:expr, $rules:expr, $path:expr, $is_dir:expr, $negate:expr) => {
            #[test]
            fn $name() {
                let rs = ruleset_from_rules($root, $rules);
                assert!($negate ^ rs.is_ignored($path, $is_dir));
            }
        };
    }

    macro_rules! not_ignored {
        ($name:ident, $root:expr, $rules:expr, $path:expr) => {
            ignored!($name, $root, $rules, $path, false, true);
        };
        ($name:ident, $root:expr, $rules:expr, $path:expr, $is_dir:expr) => {
            ignored!($name, $root, $rules, $path, $is_dir, true);
        };
    }

    const ROOT: &'static str = "/home/test/some/repo";

    ignored!(ig1, ROOT, "months", "months");
    ignored!(ig2, ROOT, "*.lock", "Cargo.lock");
    ignored!(ig3, ROOT, "*.rs", "src/main.rs");
    ignored!(ig4, ROOT, "src/*.rs", "src/main.rs");
    ignored!(ig5, ROOT, "/*.c", "cat-file.c");
    ignored!(ig6, ROOT, "/src/*.rs", "src/main.rs");
    ignored!(ig7, ROOT, "!src/main.rs\n*.rs", "src/main.rs");
    ignored!(ig8, ROOT, "foo/", "foo", true);
    ignored!(ig9, ROOT, "**/foo", "foo");
    ignored!(ig10, ROOT, "**/foo", "src/foo");
    ignored!(ig11, ROOT, "**/foo/**", "src/foo/bar");
    ignored!(ig12, ROOT, "**/foo/**", "wat/src/foo/bar/baz");
    ignored!(ig13, ROOT, "**/foo/bar", "foo/bar");
    ignored!(ig14, ROOT, "**/foo/bar", "src/foo/bar");
    ignored!(ig15, ROOT, "abc/**", "abc/x");
    ignored!(ig16, ROOT, "abc/**", "abc/x/y");
    ignored!(ig17, ROOT, "abc/**", "abc/x/y/z");
    ignored!(ig18, ROOT, "a/**/b", "a/b");
    ignored!(ig19, ROOT, "a/**/b", "a/x/b");
    ignored!(ig20, ROOT, "a/**/b", "a/x/y/b");
    ignored!(ig21, ROOT, r"\!xy", "!xy");
    ignored!(ig22, ROOT, r"\#foo", "#foo");
    ignored!(ig23, ROOT, "foo", "./foo");
    ignored!(ig24, ROOT, "target", "grep/target");
    ignored!(ig25, ROOT, "Cargo.lock", "./tabwriter-bin/Cargo.lock");
    ignored!(ig26, ROOT, "/foo/bar/baz", "./foo/bar/baz");
    ignored!(ig27, ROOT, "foo/", "xyz/foo", true);
    ignored!(ig28, ROOT, "src/*.rs", "src/grep/src/main.rs");
    ignored!(ig29, "./src", "/llvm/", "./src/llvm", true);
    ignored!(ig30, ROOT, "node_modules/ ", "node_modules", true);

    not_ignored!(ignot1, ROOT, "amonths", "months");
    not_ignored!(ignot2, ROOT, "monthsa", "months");
    not_ignored!(ignot3, ROOT, "/src/*.rs", "src/grep/src/main.rs");
    not_ignored!(ignot4, ROOT, "/*.c", "mozilla-sha1/sha1.c");
    not_ignored!(ignot5, ROOT, "/src/*.rs", "src/grep/src/main.rs");
    not_ignored!(ignot6, ROOT, "*.rs\n!src/main.rs", "src/main.rs");
    not_ignored!(ignot7, ROOT, "foo/", "foo", false);
    not_ignored!(ignot8, ROOT, "**/foo/**", "wat/src/afoo/bar/baz");
    not_ignored!(ignot9, ROOT, "**/foo/**", "wat/src/fooa/bar/baz");
    not_ignored!(ignot10, ROOT, "**/foo/bar", "foo/src/bar");
    not_ignored!(ignot11, ROOT, "#foo", "#foo");
    not_ignored!(ignot12, ROOT, "\n\n\n", "foo");
    not_ignored!(ignot13, ROOT, "foo/**", "foo", true);
    not_ignored!(
        ignot14,
        "./third_party/protobuf",
        "m4/ltoptions.m4",
        "./third_party/protobuf/csharp/src/packages/repositories.config"
    );
    not_ignored!(ignot15, ROOT, "!/bar", "foo/bar");
}

#[cfg(all(test, feature = "benchmarks"))]
mod benchmark {
    use super::RuleSet;
    use std::path::Path;
    use test::Bencher;

    const ROOT: &'static str = "/home/test/some/repo";

    // FIXME: DRY this up, perhaps with a test utils module.
    fn ruleset_from_rules<P: AsRef<Path>, S: AsRef<str>>(root: P, raw_rules: S) -> RuleSet {
        let rules: Vec<String> = raw_rules.as_ref().lines().map(|s| s.to_string()).collect();
        RuleSet::new(root, rules.iter()).unwrap()
    }

    #[bench]
    fn bench_is_ignored(b: &mut Bencher) {
        let rs = ruleset_from_rules(ROOT, "a/**/b");
        let path = Path::new("a/x/y/b");

        b.iter(|| {
            rs.is_ignored(path, false);
        })
    }
}

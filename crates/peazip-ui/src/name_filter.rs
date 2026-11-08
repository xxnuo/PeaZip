use regex::{Regex, RegexBuilder};

#[derive(Clone)]
pub(crate) struct Pattern {
    query: String,
    wildcards: bool,
    folded: String,
    compiled: Result<Option<Regex>, &'static str>,
}

impl PartialEq for Pattern {
    fn eq(&self, other: &Self) -> bool {
        self.query == other.query && self.wildcards == other.wildcards
    }
}

impl Pattern {
    pub fn new(query: &str, wildcards: bool) -> Self {
        let compiled = if query.chars().count() > 256 {
            Err("Name filter must contain at most 256 characters.")
        } else if wildcards {
            translate(&query.to_lowercase()).and_then(|expression| {
                RegexBuilder::new(&expression)
                    .dot_matches_new_line(true)
                    .size_limit(1024 * 1024)
                    .dfa_size_limit(1024 * 1024)
                    .build()
                    .map(Some)
                    .map_err(|_| "Name mask exceeds the matching limit.")
            })
        } else {
            Ok(None)
        };
        Self {
            query: query.to_owned(),
            folded: query.to_lowercase(),
            wildcards,
            compiled,
        }
    }

    pub fn matches(&self, name: &str) -> bool {
        match &self.compiled {
            Ok(Some(pattern)) => pattern.is_match(&name.to_lowercase()),
            Ok(None) => name.to_lowercase().contains(&self.folded),
            Err(_) => false,
        }
    }

    pub fn error(&self) -> Option<&'static str> {
        self.compiled.as_ref().err().copied()
    }
}

fn scalar(character: char) -> String {
    format!("\\u{{{:x}}}", u32::from(character))
}

fn translate(query: &str) -> Result<String, &'static str> {
    let chars: Vec<_> = query.chars().collect();
    let mut output = String::new();
    let mut at = 0;
    while at < chars.len() {
        match chars[at] {
            '*' => {
                if !output.ends_with(".*") {
                    output.push_str(".*");
                }
            }
            '?' => output.push('.'),
            '[' => {
                at += 1;
                output.push('[');
                if chars.get(at) == Some(&'!') {
                    output.push('^');
                    at += 1;
                }
                let mut members = 0;
                while let Some(&start) = chars.get(at) {
                    if start == ']' {
                        break;
                    }
                    at += 1;
                    if chars.get(at) == Some(&'-') {
                        at += 1;
                        let end = *chars
                            .get(at)
                            .ok_or("Invalid name mask: unclosed character set.")?;
                        at += 1;
                        let (start, end) = if start <= end {
                            (start, end)
                        } else {
                            (end, start)
                        };
                        output.push_str(&scalar(start));
                        output.push('-');
                        output.push_str(&scalar(end));
                    } else {
                        output.push_str(&scalar(start));
                    }
                    members += 1;
                }
                if chars.get(at) != Some(&']') {
                    return Err("Invalid name mask: unclosed character set.");
                }
                if members == 0 {
                    return Err("Invalid name mask: empty character set.");
                }
                output.push(']');
            }
            literal => output.push_str(&scalar(literal)),
        }
        at += 1;
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_match_unicode_substrings_and_lazarus_character_sets() {
        for (query, yes, no) in [
            ("a?c", "prefix A字c suffix", "ac"),
            ("*.txt", "archive.TXT", "archive.bin"),
            ("[z-a]", "B", "3"),
            ("[!0-9]", "字", "123"),
            ("[^a]", "^", "b"),
            ("[*]", "a*b", "abc"),
            ("[?]", "a?b", "abc"),
            ("[[]", "[", "x"),
            ("[-a]", "-", "b"),
            ("\\*", "folder\\file", "file"),
        ] {
            let pattern = Pattern::new(query, true);
            assert_eq!(pattern.error(), None, "{query}");
            assert!(pattern.matches(yes), "{query} {yes}");
            assert!(!pattern.matches(no), "{query} {no}");
        }
    }

    #[test]
    fn invalid_masks_do_not_match_and_literal_mode_does_not_parse_them() {
        for query in ["[", "[abc", "[]", "[!]", "[a-]", "[]]"] {
            let pattern = Pattern::new(query, true);
            assert!(pattern.error().is_some(), "{query}");
            assert!(!pattern.matches(query));
            assert!(Pattern::new(query, false).matches(query));
        }
        assert!(Pattern::new(&"?".repeat(257), true).error().is_some());
        assert!(Pattern::new(&"*a".repeat(128), true).matches(&"a".repeat(1024)));
    }

    #[test]
    fn lowercase_expansion_does_not_hide_raw_query_limit_changes_from_the_cache() {
        let query = "İ".repeat(256);
        let allowed = Pattern::new(&query, true);
        let oversized = Pattern::new(&query.to_lowercase(), true);
        assert!(allowed.error().is_none());
        assert!(oversized.error().is_some());
        assert!(allowed != oversized);
    }
}

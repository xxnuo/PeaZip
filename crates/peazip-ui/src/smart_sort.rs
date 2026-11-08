// Fixed PeaZip smartsortstring/smartsortable: only the first ASCII number per part.
fn number_key(value: &str) -> String {
    let Some(start) = value.bytes().position(|byte| byte.is_ascii_digit()) else {
        return value.into();
    };
    let end = start
        + value.as_bytes()[start..]
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .count();
    let significant = value[start..end].trim_start_matches('0');
    let mut key = String::with_capacity(value.len() + significant.len() * 2 + 1);
    key.push_str(&value[..start]);
    key.extend(std::iter::repeat_n('9', significant.len()));
    key.push(' ');
    key.push_str(significant);
    key.push_str(&value[start..]);
    key
}

pub(crate) fn key(name: &str) -> String {
    if name.is_empty() {
        return String::new();
    }
    let (stem, extension) = if name.contains('.') {
        let basename = name.rsplit('/').next().unwrap_or(name);
        match basename.rfind('.') {
            Some(at) if at > 0 => {
                let extension = &basename[at..];
                let stem = if extension.len() <= 6 && !extension.contains(' ') {
                    &basename[..at]
                } else {
                    basename
                };
                (stem, extension)
            }
            _ => (basename, ""),
        }
    } else {
        (name, "")
    };
    let mut result = number_key(stem);
    result.push('\'');
    result.push_str(&number_key(extension));
    result
}

pub(crate) fn comparison_key(name: &str, smart: bool) -> String {
    let mut result = if smart { key(name) } else { name.into() };
    // Match the original comparator's ASCII case-insensitivity without claiming locale equivalence.
    result.make_ascii_lowercase();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_ascii_number_uses_magnitude_without_integer_parsing() {
        assert_eq!(number_key("file00012part3"), "file99 1200012part3");
        assert_eq!(number_key("file000part3"), "file 000part3");
        assert_eq!(number_key("你好2"), "你好9 22");
        assert!(key("file2.txt") < key("file10.txt"));
        assert!(key("file02.txt") < key("file2.txt"));
        assert!(key("file0.txt") < key("file00.txt"));
        // Later runs stay lexical; using a generic natural-sort crate would change this rule.
        assert!(key("file2part10.txt") < key("file2part3.txt"));
        assert_eq!(number_key("file٢"), "file٢");
        let long = "9".repeat(1024);
        assert!(key(&format!("n{long}")) > key("n99999999999999999999"));
    }

    #[test]
    fn extension_split_keeps_upstream_short_extension_and_space_rules() {
        assert_eq!(key("file2.txt"), "file9 22'.txt");
        assert_eq!(key("report.txt2"), "report'.txt9 22");
        assert_eq!(key("report.txt100"), "report.txt999 100100'.txt999 100100");
        assert_eq!(key("name.a 2"), "name.a 9 22'.a 9 22");
        assert!(key("report.txt2") < key("report.txt10"));
        assert_eq!(key("a2.tar.gz"), "a9 22.tar'.gz");
        assert_eq!(key(".x2"), ".x9 22'");
        assert_eq!(key("trailing2."), "trailing9 22'.");
        assert_eq!(key(""), "");
        assert_eq!(key("plain"), "plain'");
        assert_eq!(
            comparison_key("File2.TXT", true),
            comparison_key("file2.txt", true)
        );
        assert_eq!(comparison_key("File10.TXT", false), "file10.txt");
    }
}

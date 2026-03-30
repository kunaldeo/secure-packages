/// Normalize a PyPI package name per PEP 503.
/// Lowercases and replaces any runs of `[-_.]` with a single `-`.
pub fn normalize_name(name: &str) -> String {
    let mut result = String::with_capacity(name.len());
    let mut prev_was_separator = false;

    for c in name.chars() {
        if c == '-' || c == '_' || c == '.' {
            if !prev_was_separator && !result.is_empty() {
                result.push('-');
            }
            prev_was_separator = true;
        } else {
            result.push(c.to_ascii_lowercase());
            prev_was_separator = false;
        }
    }

    // Trim trailing separator
    if result.ends_with('-') {
        result.pop();
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_normalization() {
        assert_eq!(normalize_name("Requests"), "requests");
        assert_eq!(normalize_name("Flask"), "flask");
    }

    #[test]
    fn test_underscores_to_hyphens() {
        assert_eq!(normalize_name("my_package"), "my-package");
        assert_eq!(normalize_name("my_cool_package"), "my-cool-package");
    }

    #[test]
    fn test_dots_to_hyphens() {
        assert_eq!(normalize_name("zope.interface"), "zope-interface");
    }

    #[test]
    fn test_mixed_separators() {
        assert_eq!(normalize_name("My_Cool.Package"), "my-cool-package");
    }

    #[test]
    fn test_consecutive_separators() {
        assert_eq!(normalize_name("my--package"), "my-package");
        assert_eq!(normalize_name("my_.package"), "my-package");
    }

    #[test]
    fn test_already_normalized() {
        assert_eq!(normalize_name("requests"), "requests");
        assert_eq!(normalize_name("my-package"), "my-package");
    }
}

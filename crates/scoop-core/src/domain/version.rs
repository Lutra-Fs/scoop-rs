#[derive(Debug, Clone, PartialEq, Eq)]
enum VersionToken {
    Number(u64),
    Text(String),
}

pub fn compare_versions(current: &str, latest: &str) -> std::cmp::Ordering {
    let current_tokens = tokenize_version(current);
    let latest_tokens = tokenize_version(latest);
    let max_len = current_tokens.len().max(latest_tokens.len());

    for index in 0..max_len {
        match (current_tokens.get(index), latest_tokens.get(index)) {
            (Some(left), Some(right)) => match compare_token(left, right) {
                std::cmp::Ordering::Equal => continue,
                ordering => return ordering,
            },
            (None, Some(VersionToken::Text(text))) if is_prerelease(text) => {
                return std::cmp::Ordering::Greater;
            }
            (Some(VersionToken::Text(text)), None) if is_prerelease(text) => {
                return std::cmp::Ordering::Less;
            }
            (None, Some(_)) => return std::cmp::Ordering::Less,
            (Some(_), None) => return std::cmp::Ordering::Greater,
            (None, None) => break,
        }
    }

    std::cmp::Ordering::Equal
}

fn tokenize_version(version: &str) -> Vec<VersionToken> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut current_is_digit = None;

    for character in version.replace('+', "-").chars() {
        if matches!(character, '.' | '-' | '_') {
            push_token(&mut tokens, &mut current, current_is_digit.take());
            continue;
        }

        let is_digit = character.is_ascii_digit();
        if current_is_digit.is_some() && current_is_digit != Some(is_digit) {
            push_token(&mut tokens, &mut current, current_is_digit.take());
        }
        current.push(character);
        current_is_digit = Some(is_digit);
    }
    push_token(&mut tokens, &mut current, current_is_digit);
    tokens
}

fn push_token(tokens: &mut Vec<VersionToken>, current: &mut String, is_digit: Option<bool>) {
    if current.is_empty() {
        return;
    }
    let token = if is_digit == Some(true) {
        VersionToken::Number(current.parse().unwrap_or(0))
    } else {
        VersionToken::Text(current.to_ascii_lowercase())
    };
    tokens.push(token);
    current.clear();
}

fn compare_token(left: &VersionToken, right: &VersionToken) -> std::cmp::Ordering {
    match (left, right) {
        (VersionToken::Number(left), VersionToken::Number(right)) => left.cmp(right),
        (VersionToken::Text(left), VersionToken::Text(right)) => left.cmp(right),
        (VersionToken::Number(_), VersionToken::Text(_)) => std::cmp::Ordering::Greater,
        (VersionToken::Text(_), VersionToken::Number(_)) => std::cmp::Ordering::Less,
    }
}

fn is_prerelease(token: &str) -> bool {
    matches!(token, "alpha" | "beta" | "rc" | "pre")
}

#[cfg(test)]
mod tests {
    use super::compare_versions;

    #[test]
    fn compares_release_and_prerelease_versions_like_scoop() {
        assert!(compare_versions("1.0.0", "1.0.1").is_lt());
        assert!(compare_versions("1.0.0-rc1", "1.0.0").is_lt());
        assert!(compare_versions("2025.10", "2025.2").is_gt());
        assert_eq!(
            compare_versions("1.0.0", "1.0.0"),
            std::cmp::Ordering::Equal
        );
    }
}

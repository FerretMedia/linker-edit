//! How full the agent's context is, and when to offer compacting: src/lib/context.ts.

use chrono::{DateTime, Utc};
use linker_edit_protocol::{COMPACT_SUGGESTION_MIN_TOKENS, ContextUsage, Provider};

/// 84000 → "84K", 1200000 → "1.2M".
pub fn format_tokens(tokens: u32) -> String {
    if tokens >= 1_000_000 {
        // Number(x.toFixed(1)): one decimal, without a trailing ".0".
        let millions = (tokens as f64 / 100_000.).round() / 10.;
        format!("{millions}M")
    } else if tokens >= 1_000 {
        format!("{}K", (tokens as f64 / 1_000.).round())
    } else {
        tokens.to_string()
    }
}

/// `toLocaleString("en-US")`: 84000 → "84,000".
pub fn group_thousands(value: u32) -> String {
    let digits = value.to_string();
    let mut out = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// When Claude's prompt cache expires, if the server said.
pub fn cache_expiry(context: Option<&ContextUsage>) -> Option<DateTime<Utc>> {
    let text = context?.cache_expires_at.as_deref()?;
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|time| time.with_timezone(&Utc))
}

/// Whether to offer compacting before the next message: the chat is big and Claude's prompt cache
/// has expired, so the next turn would reread all of it uncached anyway.
pub fn compact_suggested(
    provider: Provider,
    context: Option<&ContextUsage>,
    now: DateTime<Utc>,
) -> bool {
    if provider != Provider::Claude {
        return false;
    }
    let (Some(context), Some(expires)) = (context, cache_expiry(context)) else {
        return false;
    };
    context.used_tokens >= COMPACT_SUGGESTION_MIN_TOKENS && now >= expires
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeDelta;

    // context.test.ts
    #[test]
    fn suggests_compacting_a_big_claude_chat_whose_cache_expired() {
        let expires_at = "2026-11-08T12:00:00Z";
        let after: DateTime<Utc> = expires_at.parse().unwrap();
        let before = after - TimeDelta::milliseconds(1);
        let big = ContextUsage {
            used_tokens: 250_000,
            max_tokens: Some(1_000_000),
            cache_expires_at: Some(expires_at.into()),
        };
        let small = ContextUsage {
            used_tokens: 150_000,
            ..big.clone()
        };
        let unknown = ContextUsage {
            cache_expires_at: None,
            ..big.clone()
        };
        let cases = [
            (
                "a big Claude chat whose cache expired",
                Provider::Claude,
                &big,
                after,
                true,
            ),
            (
                "while the cache is still warm",
                Provider::Claude,
                &big,
                before,
                false,
            ),
            ("a small chat", Provider::Claude, &small, after, false),
            (
                "an unknown cache lifetime",
                Provider::Claude,
                &unknown,
                after,
                false,
            ),
            ("Codex", Provider::Codex, &big, after, false),
        ];
        for (name, provider, context, now, expected) in cases {
            assert_eq!(
                compact_suggested(provider, Some(context), now),
                expected,
                "{name}"
            );
        }
        assert!(!compact_suggested(Provider::Claude, None, after));
    }

    #[test]
    fn formats_tokens() {
        for (tokens, label) in [
            (950, "950"),
            (84_400, "84K"),
            (1_000_000, "1M"),
            (1_250_000, "1.3M"),
        ] {
            assert_eq!(format_tokens(tokens), label);
        }
        assert_eq!(group_thousands(1_000_000), "1,000,000");
        assert_eq!(group_thousands(45_210), "45,210");
        assert_eq!(group_thousands(950), "950");
    }
}

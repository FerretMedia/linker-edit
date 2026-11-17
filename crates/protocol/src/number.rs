// Numbers as JSON.stringify writes them: a whole f64 goes out as 40, not 40.0, so what the Rust server sends is
// the same JSON the TypeScript server sent.

use serde::Serializer;

/// Whole numbers JavaScript still holds exactly.
const MAX_SAFE: f64 = 9_007_199_254_740_991.0;

pub(crate) fn serialize<S: Serializer>(value: &f64, serializer: S) -> Result<S::Ok, S::Error> {
    if value.fract() == 0.0 && value.abs() <= MAX_SAFE {
        // -0 is 0 in JSON.stringify too.
        serializer.serialize_i64(*value as i64)
    } else {
        serializer.serialize_f64(*value)
    }
}

pub(crate) fn option<S: Serializer>(value: &Option<f64>, serializer: S) -> Result<S::Ok, S::Error> {
    match value {
        Some(value) => serialize(value, serializer),
        None => serializer.serialize_none(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::PreviewState;

    #[test]
    fn writes_whole_numbers_like_javascript() {
        let rendering = |percent| {
            serde_json::to_value(PreviewState::Rendering {
                timeline: None,
                percent,
            })
            .unwrap()
        };
        assert_eq!(
            rendering(40.0),
            json!({ "state": "rendering", "timeline": null, "percent": 40 })
        );
        assert_eq!(rendering(-0.0)["percent"].to_string(), "0");
        assert_eq!(rendering(12.5)["percent"].to_string(), "12.5");
        // JSON.stringify writes NaN as null.
        assert_eq!(rendering(f64::NAN)["percent"], json!(null));
    }
}

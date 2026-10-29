//! Quiz scoring, kept pure so every rule is unit-tested: the keys learners see instead of option
//! ids, which responses are well-formed, what counts as correct for each question type, and the
//! exact score and pass decision.

use rust_decimal::{Decimal, RoundingStrategy};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use uuid::Uuid;

pub const SINGLE: &str = "Single Choice";
pub const MULTIPLE: &str = "Multiple Choice";
pub const TRUE_FALSE: &str = "True/False";
pub const SHORT: &str = "Short Answer";

/// Longest short answer a learner may send.
pub const MAX_ANSWER: usize = 500;

/// The key an option is shown under in one attempt. Option ids are time-ordered and would reveal
/// the order the author wrote the options in; a keyed hash of attempt and option reveals nothing
/// and still maps back on the server.
pub fn option_key(attempt: Uuid, option: Uuid) -> String {
    let mut h = Sha256::new();
    h.update(b"ferrum-lms-option\0");
    h.update(attempt.as_bytes());
    h.update(option.as_bytes());
    hex::encode(&h.finalize()[..8])
}

/// The answer key of one question in one attempt.
#[derive(Debug, Clone)]
pub struct Key {
    pub qtype: String,
    /// Keys of the correct options (choice questions).
    pub correct: BTreeSet<String>,
    /// Accepted short answers.
    pub accepted: Vec<String>,
    pub case_sensitive: bool,
}

/// A short answer as compared: trimmed, inner whitespace collapsed, lower-case unless the question
/// is case-sensitive.
pub fn normalize_answer(v: &str, case_sensitive: bool) -> String {
    let joined = v.split_whitespace().collect::<Vec<_>>().join(" ");
    if case_sensitive { joined } else { joined.to_lowercase() }
}

/// Checks a learner's response against the options shown and returns its stored form: `null`
/// (unanswered), a list of option keys in the order shown, or the short answer text.
pub fn parse_response(qtype: &str, shown: &[String], v: &Value) -> Result<Value, String> {
    if qtype == SHORT {
        return match v {
            Value::Null => Ok(Value::Null),
            Value::String(t) => {
                let t = t.trim();
                if t.chars().count() > MAX_ANSWER {
                    Err(format!("answers are limited to {MAX_ANSWER} characters"))
                } else if t.chars().any(|c| c.is_control() && c != '\n' && c != '\t') {
                    Err("the answer contains control characters".into())
                } else if t.is_empty() {
                    Ok(Value::Null)
                } else {
                    Ok(json!(t))
                }
            }
            _ => Err("answer with text".into()),
        };
    }
    let picked: Vec<&str> = match v {
        Value::Null => vec![],
        Value::String(k) => vec![k.as_str()],
        Value::Array(a) => a.iter().map(|x| x.as_str().ok_or("option keys are strings")).collect::<Result<_, _>>()?,
        _ => return Err("answer with option keys".into()),
    };
    if let Some(bad) = picked.iter().find(|k| !shown.iter().any(|s| s == *k)) {
        return Err(format!("'{}' is not one of this question's options", crate::util::cut(bad, 40)));
    }
    let set: BTreeSet<&str> = picked.iter().copied().collect();
    if set.len() != picked.len() {
        return Err("an option is chosen twice".into());
    }
    if qtype != MULTIPLE && set.len() > 1 {
        return Err("choose one option".into());
    }
    if set.is_empty() {
        return Ok(Value::Null);
    }
    Ok(Value::Array(shown.iter().filter(|s| set.contains(s.as_str())).map(|s| json!(s)).collect()))
}

/// Whether a stored response is right: exactly the one correct option, exactly the set of correct
/// options (no partial credit), or an accepted short answer.
pub fn is_correct(key: &Key, response: &Value) -> bool {
    match key.qtype.as_str() {
        SHORT => {
            let Some(given) = response.as_str() else { return false };
            let given = normalize_answer(given, key.case_sensitive);
            !given.is_empty() && key.accepted.iter().any(|a| normalize_answer(a, key.case_sensitive) == given)
        }
        _ => {
            let chosen: BTreeSet<String> = response.as_array().map(|a| a.iter().filter_map(|k| k.as_str().map(str::to_string)).collect()).unwrap_or_default();
            let single = key.qtype != MULTIPLE;
            !chosen.is_empty() && chosen == key.correct && (!single || chosen.len() == 1)
        }
    }
}

/// Score as a percentage of the maximum, rounded half away from zero to 2 places (for display).
pub fn percentage(score: Decimal, max: Decimal) -> Decimal {
    if max.is_zero() {
        return Decimal::ZERO;
    }
    let mut p = (score * Decimal::ONE_HUNDRED / max).round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero);
    p.rescale(2);
    p
}

/// Pass decision, compared exactly (score / max ≥ passing %), never on the rounded percentage.
pub fn passed(score: Decimal, max: Decimal, passing_percentage: Decimal) -> bool {
    !max.is_zero() && score * Decimal::ONE_HUNDRED >= passing_percentage * max
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn n(v: &str) -> Decimal {
        Decimal::from_str(v).unwrap()
    }

    fn key(qtype: &str, correct: &[&str], accepted: &[&str], case_sensitive: bool) -> Key {
        Key {
            qtype: qtype.into(),
            correct: correct.iter().map(|s| s.to_string()).collect(),
            accepted: accepted.iter().map(|s| s.to_string()).collect(),
            case_sensitive,
        }
    }

    #[test]
    fn option_keys_are_stable_and_opaque() {
        let (a, o1, o2) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        assert_eq!(option_key(a, o1), option_key(a, o1));
        assert_ne!(option_key(a, o1), option_key(a, o2));
        assert_ne!(option_key(a, o1), option_key(Uuid::now_v7(), o1), "keys differ per attempt");
        assert_eq!(option_key(a, o1).len(), 16);
        assert!(!option_key(a, o1).contains(&o1.simple().to_string()[..8]));
    }

    #[test]
    fn responses_are_checked_against_what_was_shown() {
        let shown = vec!["k1".to_string(), "k2".to_string(), "k3".to_string()];
        assert_eq!(parse_response(SINGLE, &shown, &json!("k2")).unwrap(), json!(["k2"]));
        assert_eq!(parse_response(SINGLE, &shown, &json!(["k3"])).unwrap(), json!(["k3"]));
        assert_eq!(parse_response(SINGLE, &shown, &json!([])).unwrap(), Value::Null);
        assert!(parse_response(SINGLE, &shown, &json!(["k1", "k2"])).is_err());
        assert!(parse_response(TRUE_FALSE, &shown, &json!("nope")).is_err());
        assert_eq!(parse_response(MULTIPLE, &shown, &json!(["k3", "k1"])).unwrap(), json!(["k1", "k3"]), "stored in the order shown");
        assert!(parse_response(MULTIPLE, &shown, &json!(["k1", "k1"])).is_err());
        assert!(parse_response(MULTIPLE, &shown, &json!(1)).is_err());
        assert_eq!(parse_response(SHORT, &[], &json!("  Paris ")).unwrap(), json!("Paris"));
        assert_eq!(parse_response(SHORT, &[], &json!("   ")).unwrap(), Value::Null);
        assert!(parse_response(SHORT, &[], &json!(["Paris"])).is_err());
        assert!(parse_response(SHORT, &[], &json!("x".repeat(MAX_ANSWER + 1))).is_err());
    }

    #[test]
    fn correctness_per_question_type() {
        let single = key(SINGLE, &["b"], &[], false);
        assert!(is_correct(&single, &json!(["b"])));
        assert!(!is_correct(&single, &json!(["a"])) && !is_correct(&single, &Value::Null) && !is_correct(&single, &json!(["a", "b"])));
        let multi = key(MULTIPLE, &["a", "c"], &[], false);
        assert!(is_correct(&multi, &json!(["a", "c"])));
        assert!(!is_correct(&multi, &json!(["a"])), "no partial credit");
        assert!(!is_correct(&multi, &json!(["a", "b", "c"])), "an extra wrong option fails");
        let tf = key(TRUE_FALSE, &["t"], &[], false);
        assert!(is_correct(&tf, &json!(["t"])) && !is_correct(&tf, &json!(["f"])));
        let short = key(SHORT, &[], &["Mitochondria", "the mitochondrion"], false);
        assert!(is_correct(&short, &json!(" mitochondria ")));
        assert!(is_correct(&short, &json!("The   Mitochondrion")));
        assert!(!is_correct(&short, &json!("mitochondrias")) && !is_correct(&short, &json!("")) && !is_correct(&short, &Value::Null));
        let strict = key(SHORT, &[], &["NaCl"], true);
        assert!(is_correct(&strict, &json!("NaCl")) && !is_correct(&strict, &json!("nacl")));
    }

    #[test]
    fn exact_scores_and_pass_marks() {
        assert_eq!(percentage(n("3.5"), n("5")), n("70.00"));
        assert!(passed(n("3.5"), n("5"), n("70")), "exactly on the mark passes");
        assert!(!passed(n("3.49"), n("5"), n("70")));
        assert_eq!(percentage(n("2"), n("3")), n("66.67"));
        assert!(!passed(n("2"), n("3"), n("66.67")), "66.666…% is below 66.67% even though it displays as 66.67");
        assert!(passed(n("2"), n("3"), n("66.66")));
        assert_eq!(percentage(n("1"), n("8")), n("12.50"));
        assert_eq!(percentage(n("0"), n("0")), Decimal::ZERO);
        assert!(!passed(n("0"), n("0"), n("0")));
        assert!(passed(n("0"), n("4"), n("0")), "a 0% pass mark passes everyone");
    }
}

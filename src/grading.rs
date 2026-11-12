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
pub const NUMERIC: &str = "Numeric";
pub const ORDERING: &str = "Ordering";
pub const MATCHING: &str = "Matching";
pub const BLANKS: &str = "Fill in the Blanks";
pub const ESSAY: &str = "Essay";
pub const FILE: &str = "File Upload";

/// Question types a person grades: their answers wait for a grader, and the attempt's result is
/// final only once every one is graded.
pub fn by_person(qtype: &str) -> bool {
    qtype == ESSAY || qtype == FILE
}

/// Longest essay a learner may send.
pub const MAX_ESSAY: usize = 50_000;

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

/// The key a matching question's right-hand answer is shown under: another keyed hash, so the
/// keys of a pair never tell each other.
pub fn match_key(attempt: Uuid, option: Uuid) -> String {
    let mut h = Sha256::new();
    h.update(b"ferrum-lms-match\0");
    h.update(attempt.as_bytes());
    h.update(option.as_bytes());
    hex::encode(&h.finalize()[..8])
}

/// The blanks a fill-in question's text holds: `[[1]]`, `[[2]]`, … numbered from 1, each once.
pub fn blanks_in(text: &str) -> usize {
    let mut n = 0;
    while text.contains(&format!("[[{}]]", n + 1)) {
        n += 1;
    }
    n
}

/// The answer key of one question in one attempt.
#[derive(Debug, Clone, Default)]
pub struct Key {
    pub qtype: String,
    /// Keys of the correct options (choice questions).
    pub correct: BTreeSet<String>,
    /// Accepted short answers.
    pub accepted: Vec<String>,
    /// Short answers matched as patterns: `*` any text, `?` one character.
    pub patterns: Vec<String>,
    pub case_sensitive: bool,
    /// Ordering: the option keys in the right order.
    pub order: Vec<String>,
    /// Matching: each left key's right key.
    pub pairs: std::collections::BTreeMap<String, String>,
    /// Fill in the blanks: per blank, its accepted answers and patterns.
    pub blanks: Vec<(Vec<String>, Vec<String>)>,
    /// Numeric: the answer and how far from it still counts.
    pub numeric: Option<(Decimal, Decimal)>,
}

/// Whether `text` matches `pattern`, where `*` stands for any text and `?` for one character.
pub fn wildcard(pattern: &str, text: &str) -> bool {
    let (p, t): (Vec<char>, Vec<char>) = (pattern.chars().collect(), text.chars().collect());
    let (mut pi, mut ti, mut star, mut mark) = (0, 0, None, 0);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(sp) = star {
            pi = sp + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|c| *c == '*')
}

/// Whether a text answer is one of the accepted ones or matches a pattern.
fn text_right(given: &str, accepted: &[String], patterns: &[String], case_sensitive: bool) -> bool {
    let given = normalize_answer(given, case_sensitive);
    !given.is_empty()
        && (accepted.iter().any(|a| normalize_answer(a, case_sensitive) == given)
            || patterns.iter().any(|p| wildcard(&normalize_answer(p, case_sensitive), &given)))
}

/// A number as a learner may write it: `3.5`, `-2`, `1e3` is refused (digits, one point, a sign).
pub fn parse_number(v: &Value) -> Option<Decimal> {
    match v {
        Value::Number(n) => n.to_string().parse().ok(),
        Value::String(t) => {
            let t = t.trim().replace(',', ".");
            (!t.is_empty() && t.len() <= 40).then(|| t.parse::<Decimal>().ok()).flatten()
        }
        _ => None,
    }
}

/// A short answer as compared: trimmed, inner whitespace collapsed, lower-case unless the question
/// is case-sensitive.
pub fn normalize_answer(v: &str, case_sensitive: bool) -> String {
    let joined = v.split_whitespace().collect::<Vec<_>>().join(" ");
    if case_sensitive { joined } else { joined.to_lowercase() }
}

/// Checks a learner's response against the options shown and returns its stored form: `null`
/// (unanswered), a list of option keys in the order shown, or the short answer text.
#[cfg(test)]
pub fn parse_response(qtype: &str, shown: &[String], v: &Value) -> Result<Value, String> {
    parse_response_of(qtype, shown, &[], 0, v)
}

/// [`parse_response`] for every type: `right` are a matching question's right-hand keys, `blanks`
/// a fill-in question's number of blanks. Stored forms: numeric, the number as text; ordering, every
/// key in the learner's order; matching, `{left key: right key}`; blanks, a list of texts; essay, the
/// text; file upload, the uploaded file's `/files/{id}` path (whose it is is checked by the caller).
pub fn parse_response_of(qtype: &str, shown: &[String], right: &[String], blanks: usize, v: &Value) -> Result<Value, String> {
    let text = |t: &str, max: usize| -> Result<Value, String> {
        let t = t.trim();
        if t.chars().count() > max {
            Err(format!("answers are limited to {max} characters"))
        } else if t.chars().any(|c| c.is_control() && c != '\n' && c != '\t' && c != '\r') {
            Err("the answer contains control characters".into())
        } else if t.is_empty() {
            Ok(Value::Null)
        } else {
            Ok(json!(t))
        }
    };
    match (qtype, v) {
        (_, Value::Null) => return Ok(Value::Null),
        (NUMERIC, _) => return parse_number(v).map(|n| json!(n.normalize().to_string())).ok_or_else(|| "answer with a number".into()),
        (ESSAY, Value::String(t)) => return text(t, MAX_ESSAY),
        (FILE, Value::String(t)) => {
            return match crate::util::file_id(t.trim()) {
                Some(f) => Ok(json!(format!("/files/{f}"))),
                None => Err("answer with a file uploaded with /api/files".into()),
            };
        }
        (ESSAY | FILE, _) => return Err("answer with text".into()),
        (ORDERING, Value::Array(a)) => {
            let keys: Vec<&str> = a.iter().map(|x| x.as_str().ok_or("option keys are strings")).collect::<Result<_, _>>()?;
            let set: BTreeSet<&str> = keys.iter().copied().collect();
            if keys.len() != shown.len() || set.len() != keys.len() || !keys.iter().all(|k| shown.iter().any(|s| s == k)) {
                return Err("put every option of this question in order, each once".into());
            }
            return Ok(json!(keys));
        }
        (ORDERING, _) => return Err("answer with the option keys in order".into()),
        (MATCHING, Value::Object(m)) => {
            let mut out = serde_json::Map::new();
            for (left, r) in m {
                let r = r.as_str().ok_or("match each item with an answer's key")?;
                if !shown.iter().any(|s| s == left) || !right.iter().any(|x| x == r) {
                    return Err(format!("'{}' is not one of this question's items or answers", crate::util::cut(left, 40)));
                }
                out.insert(left.clone(), json!(r));
            }
            return Ok(if out.is_empty() { Value::Null } else { Value::Object(out) });
        }
        (MATCHING, _) => return Err("answer with an object of item key → answer key".into()),
        (BLANKS, Value::Array(a)) => {
            if a.len() != blanks {
                return Err(format!("this question has {blanks} blanks"));
            }
            let mut out = Vec::with_capacity(a.len());
            for x in a {
                out.push(match x {
                    Value::Null => Value::Null,
                    Value::String(t) => text(t, MAX_ANSWER)?,
                    _ => return Err("fill each blank with text".into()),
                });
            }
            return Ok(if out.iter().all(Value::is_null) { Value::Null } else { Value::Array(out) });
        }
        (BLANKS, _) => return Err("answer with a list of texts, one a blank".into()),
        _ => {}
    }
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
#[cfg(test)]
pub fn is_correct(key: &Key, response: &Value) -> bool {
    fraction(key, response) == Decimal::ONE
}

/// How much of a stored response is right, from 0 to 1: whole or nothing for single choice,
/// true/false, short and numeric answers; for multiple choice the right options chosen less the wrong
/// ones, over the right ones; for ordering the places right, for matching the pairs right and for
/// blanks the blanks right, over all of them. Questions a person grades are 0 here.
pub fn fraction(key: &Key, response: &Value) -> Decimal {
    let whole = |ok: bool| if ok { Decimal::ONE } else { Decimal::ZERO };
    let share = |right: usize, of: usize| if of == 0 { Decimal::ZERO } else { Decimal::from(right as u64) / Decimal::from(of as u64) };
    match key.qtype.as_str() {
        SHORT => whole(response.as_str().is_some_and(|g| text_right(g, &key.accepted, &key.patterns, key.case_sensitive))),
        NUMERIC => whole(match (parse_number(response), key.numeric) {
            (Some(x), Some((answer, tolerance))) => (x - answer).abs() <= tolerance.abs(),
            _ => false,
        }),
        ORDERING => {
            let given: Vec<&str> = response.as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
            if given.len() != key.order.len() {
                return Decimal::ZERO;
            }
            share(given.iter().zip(&key.order).filter(|(g, o)| **g == o.as_str()).count(), key.order.len())
        }
        MATCHING => {
            let right = key.pairs.iter().filter(|(l, r)| response.get(l.as_str()).and_then(Value::as_str) == Some(r.as_str())).count();
            share(right, key.pairs.len())
        }
        BLANKS => {
            let given = response.as_array().cloned().unwrap_or_default();
            let right = key
                .blanks
                .iter()
                .enumerate()
                .filter(|(i, (acc, pat))| given.get(*i).and_then(Value::as_str).is_some_and(|g| text_right(g, acc, pat, key.case_sensitive)))
                .count();
            share(right, key.blanks.len())
        }
        ESSAY | FILE => Decimal::ZERO,
        _ => {
            let chosen: BTreeSet<String> = response.as_array().map(|a| a.iter().filter_map(|k| k.as_str().map(str::to_string)).collect()).unwrap_or_default();
            if key.qtype != MULTIPLE {
                return whole(chosen.len() == 1 && chosen == key.correct);
            }
            if chosen.is_empty() || key.correct.is_empty() {
                return Decimal::ZERO;
            }
            let right = chosen.intersection(&key.correct).count() as i64;
            let wrong = chosen.len() as i64 - right;
            share((right - wrong).max(0) as usize, key.correct.len())
        }
    }
}

/// Points for an answer of `points` that is `fraction` right: all of them when wholly right; with
/// partial credit, that share of them (to the cent, half away from zero); otherwise none, and with
/// negative marking an answered question that is not wholly right loses `negative_percent` of its
/// points. (A whole attempt never scores below zero: see [`attempt_score`].)
pub fn award(points: Decimal, fraction: Decimal, answered: bool, partial: bool, negative_percent: Decimal) -> Decimal {
    if fraction >= Decimal::ONE {
        return points;
    }
    if partial && fraction > Decimal::ZERO {
        return (points * fraction).round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero);
    }
    if answered && negative_percent > Decimal::ZERO {
        return -(points * negative_percent / Decimal::ONE_HUNDRED).round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero);
    }
    Decimal::ZERO
}

/// An attempt's score from its answers' points: never below zero.
pub fn attempt_score(awarded: impl IntoIterator<Item = Decimal>) -> Decimal {
    awarded.into_iter().sum::<Decimal>().max(Decimal::ZERO)
}

/// A rubric's levels: (criterion, level, points).
pub type Levels = Vec<(String, String, Decimal)>;

/// The most a rubric gives: the best level of each criterion, added up.
pub fn rubric_max(criteria: &[String], levels: &Levels) -> Decimal {
    criteria.iter().map(|c| levels.iter().filter(|l| &l.0 == c).map(|l| l.2).max().unwrap_or_default()).sum()
}

/// A filled rubric: the level chosen for every criterion (`chosen`: criterion → level), as rows
/// `{criterion, level, points}`, and the points out of `out_of` (the rubric's own points scaled to it,
/// to the cent).
pub fn fill_rubric(criteria: &[String], levels: &Levels, chosen: &Value, out_of: Decimal) -> Result<(Value, Decimal), String> {
    let Value::Object(map) = chosen else { return Err("choose a level for each criterion: {criterion: level}".into()) };
    if let Some(extra) = map.keys().find(|k| !criteria.contains(k)) {
        return Err(format!("{} is not a criterion of this rubric", crate::util::cut(extra, 60)));
    }
    let mut rows = Vec::with_capacity(criteria.len());
    let mut raw = Decimal::ZERO;
    for c in criteria {
        let level = map.get(c).and_then(Value::as_str).ok_or_else(|| format!("choose a level for {c}"))?;
        let found = levels.iter().find(|l| &l.0 == c && l.1 == level).ok_or_else(|| format!("{level} is not a level of {c}"))?;
        raw += found.2;
        rows.push(json!({"criterion": c, "level": level, "points": found.2.normalize().to_string()}));
    }
    let max = rubric_max(criteria, levels);
    let scaled = if max.is_zero() { Decimal::ZERO } else { (raw * out_of / max).round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero) };
    Ok((json!({"criteria": rows, "points": raw.normalize().to_string(), "max": max.normalize().to_string()}), scaled))
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
            ..Default::default()
        }
    }

    fn strs(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_new_question_types() {
        // Patterns.
        assert!(wildcard("colo*r", "colour") && wildcard("colo*r", "color") && wildcard("c?t", "cat") && !wildcard("c?t", "cart"));
        assert!(wildcard("*", "") && wildcard("a*b*c", "aXXbYc") && !wildcard("a*b", "ac"));
        let short = Key { qtype: SHORT.into(), accepted: strs(&["grey"]), patterns: strs(&["gr?y", "colo*r"]), ..Default::default() };
        assert!(is_correct(&short, &json!("Gray")) && is_correct(&short, &json!("colour")) && !is_correct(&short, &json!("green")));
        // Numbers within a tolerance.
        let num = Key { qtype: NUMERIC.into(), numeric: Some((n("9.81"), n("0.05"))), ..Default::default() };
        assert!(is_correct(&num, &json!("9.8")) && is_correct(&num, &json!(9.86)) && !is_correct(&num, &json!("9.87")) && !is_correct(&num, &json!("x")));
        assert_eq!(parse_response_of(NUMERIC, &[], &[], 0, &json!(" 9,80 ")).unwrap(), json!("9.8"));
        assert!(parse_response_of(NUMERIC, &[], &[], 0, &json!("nine")).is_err());
        // Ordering: every key once; places right.
        let shown = strs(&["a", "b", "c", "d"]);
        let ord = Key { qtype: ORDERING.into(), order: strs(&["a", "b", "c", "d"]), ..Default::default() };
        assert!(parse_response_of(ORDERING, &shown, &[], 0, &json!(["a", "b"])).is_err(), "all of them");
        assert!(parse_response_of(ORDERING, &shown, &[], 0, &json!(["a", "a", "b", "c"])).is_err(), "each once");
        assert_eq!(fraction(&ord, &json!(["a", "b", "d", "c"])), n("0.5"));
        assert!(is_correct(&ord, &json!(["a", "b", "c", "d"])));
        // Matching: pairs right.
        let mat = Key {
            qtype: MATCHING.into(),
            pairs: [("l1", "r1"), ("l2", "r2"), ("l3", "r3")].iter().map(|(a, b)| (a.to_string(), b.to_string())).collect(),
            ..Default::default()
        };
        let right = strs(&["r1", "r2", "r3"]);
        assert!(parse_response_of(MATCHING, &strs(&["l1", "l2", "l3"]), &right, 0, &json!({"l1": "zz"})).is_err());
        assert_eq!(fraction(&mat, &json!({"l1": "r1", "l2": "r3", "l3": "r2"})).round_dp(4), n("0.3333"));
        // Blanks.
        assert_eq!(blanks_in("The [[1]] of France is [[2]]."), 2);
        assert_eq!(blanks_in("No blanks [[2]]"), 0, "numbered from 1");
        let bl = Key { qtype: BLANKS.into(), blanks: vec![(strs(&["capital"]), vec![]), (strs(&["Paris"]), strs(&["par*"]))], ..Default::default() };
        assert!(parse_response_of(BLANKS, &[], &[], 2, &json!(["capital"])).is_err(), "one a blank");
        assert_eq!(fraction(&bl, &json!(["capital", "nice"])), n("0.5"));
        assert!(is_correct(&bl, &json!(["Capital", "parigi"])));
        // A person grades essays and files.
        assert!(by_person(ESSAY) && by_person(FILE) && !by_person(SHORT));
        assert!(parse_response_of(FILE, &[], &[], 0, &json!("/etc/passwd")).is_err());
        let f = Uuid::now_v7();
        assert_eq!(parse_response_of(FILE, &[], &[], 0, &json!(format!("/files/{f}"))).unwrap(), json!(format!("/files/{f}")));
        assert_eq!(fraction(&Key { qtype: ESSAY.into(), ..Default::default() }, &json!("anything")), Decimal::ZERO);
    }

    #[test]
    fn partial_credit_and_negative_marking() {
        let multi = key(MULTIPLE, &["a", "b", "c"], &[], false);
        assert_eq!(fraction(&multi, &json!(["a", "b"])).round_dp(4), n("0.6667"));
        assert_eq!(fraction(&multi, &json!(["a", "x"])), Decimal::ZERO, "a wrong pick cancels a right one");
        assert_eq!(award(n("3"), n("0.6667"), true, true, n("0")), n("2"), "two thirds of 3, to the cent");
        assert_eq!(award(n("3"), n("0.6667"), true, false, n("0")), n("0"), "all or nothing without partial credit");
        assert_eq!(award(n("2"), n("0"), true, false, n("25")), n("-0.5"), "a wrong answer loses a quarter");
        assert_eq!(award(n("2"), n("0"), false, false, n("25")), n("0"), "an unanswered one loses nothing");
        assert_eq!(award(n("2"), n("1"), true, false, n("25")), n("2"));
        assert_eq!(attempt_score([n("-0.5"), n("-0.5")]), n("0"), "never below zero");
        assert_eq!(attempt_score([n("2"), n("-0.5")]), n("1.5"));
    }

    #[test]
    fn rubrics() {
        let criteria = strs(&["Argument", "Style"]);
        let levels: Levels = vec![
            ("Argument".into(), "Strong".into(), n("6")),
            ("Argument".into(), "Weak".into(), n("2")),
            ("Style".into(), "Clear".into(), n("4")),
            ("Style".into(), "Muddled".into(), n("1")),
        ];
        assert_eq!(rubric_max(&criteria, &levels), n("10"));
        let (filled, points) = fill_rubric(&criteria, &levels, &json!({"Argument": "Strong", "Style": "Muddled"}), n("20")).unwrap();
        assert_eq!(points, n("14"), "7 of 10, out of 20");
        assert_eq!(filled["criteria"][1], json!({"criterion": "Style", "level": "Muddled", "points": "1"}));
        assert!(fill_rubric(&criteria, &levels, &json!({"Argument": "Strong"}), n("10")).unwrap_err().contains("Style"), "every criterion");
        assert!(fill_rubric(&criteria, &levels, &json!({"Argument": "Great", "Style": "Clear"}), n("10")).is_err());
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

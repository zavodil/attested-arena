//! `lines`: write a few lines of text that obey exact rules.
//!
//! A language model writes a topic and a short text about it. Code reads the
//! rules off that text — how many lines it has, the letters its lines begin
//! with, how many words one line has, a common letter one line avoids, how
//! often a word occurs, the last word — and renders the statement from fixed
//! templates. So the author's text satisfies the rules by construction, the
//! statement says exactly what the checker checks, and no model wrote either
//! the rules or the statement.
//!
//! | rule | refuses a task when |
//! |---|---|
//! | validator | the topic or the rules are out of bounds, or the statement is not the one the rules render to |
//! | witness | the author's text breaks a rule |
//! | mutations | the author's text with its last line dropped, or with its lines in reverse order, is accepted |
//!
//! Words, lines and letters are defined in the statement itself, and the
//! checker follows that definition to the letter.

use super::seating::check_salt;
use super::{json_object, Rejection};
use crate::{Rng, Verdict};
use serde::{Deserialize, Serialize};

pub const MIN_LINES: usize = 4;
pub const MAX_LINES: usize = 6;
/// Rules besides the line count and the acrostic.
pub const EXTRA_RULES: usize = 3;
/// Letters a line may be asked to avoid, most common first.
const COMMON_LETTERS: &[u8] = b"etaoinshrl";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Rule {
    /// The text has exactly `n` lines.
    LineCount { n: u8 },
    /// The first letters of the lines, top to bottom, spell `letters`.
    Acrostic { letters: String },
    /// Line `line` (1-based) has exactly `n` words.
    WordsInLine { line: u8, n: u8 },
    /// Line `line` does not contain `letter`, in either case.
    NoLetter { line: u8, letter: String },
    /// `word` occurs exactly `n` times in the text, in any case.
    WordCount { word: String, n: u8 },
    /// The last word of the text is `word`, in any case.
    LastWord { word: String },
}

impl Rule {
    fn render(&self) -> String {
        match self {
            Rule::LineCount { n } => format!("The text has exactly {n} lines."),
            Rule::Acrostic { letters } => {
                format!("Read from top to bottom, the first letters of the lines spell \"{letters}\".")
            }
            Rule::WordsInLine { line, n } => format!("Line {line} has exactly {n} words."),
            Rule::NoLetter { line, letter } => format!("Line {line} does not contain the letter \"{letter}\", in either case."),
            Rule::WordCount { word, n } => {
                let times = if *n == 1 { "time" } else { "times" };
                format!("The word \"{word}\" appears exactly {n} {times} in the text, in any case.")
            }
            Rule::LastWord { word } => format!("The last word of the text is \"{word}\"."),
        }
    }

    fn check(&self, text: &Text) -> Result<(), String> {
        match self {
            Rule::LineCount { n } => {
                let got = text.lines.len();
                (got == *n as usize).then_some(()).ok_or_else(|| format!("the text has {got} lines, not {n}"))
            }
            Rule::Acrostic { letters } => {
                let got: String = text.lines.iter().map(|l| l.first_letter.map(|c| c.to_ascii_uppercase()).unwrap_or('?')).collect();
                (got == *letters).then_some(()).ok_or_else(|| format!("the lines begin with \"{got}\", not \"{letters}\""))
            }
            Rule::WordsInLine { line, n } => {
                let got = text.line(*line)?.words.len();
                (got == *n as usize).then_some(()).ok_or_else(|| format!("line {line} has {got} words, not {n}"))
            }
            Rule::NoLetter { line, letter } => {
                let l = text.line(*line)?;
                let c = letter.chars().next().unwrap_or(' ');
                (!l.raw.chars().any(|x| x.eq_ignore_ascii_case(&c)))
                    .then_some(())
                    .ok_or_else(|| format!("line {line} contains the letter \"{letter}\""))
            }
            Rule::WordCount { word, n } => {
                let got = text.lines.iter().flat_map(|l| &l.words).filter(|w| *w == word).count();
                (got == *n as usize).then_some(()).ok_or_else(|| format!("\"{word}\" appears {got} times, not {n}"))
            }
            Rule::LastWord { word } => {
                let got = text.lines.last().and_then(|l| l.words.last()).map(String::as_str).unwrap_or("");
                (got == word).then_some(()).ok_or_else(|| format!("the last word is \"{got}\", not \"{word}\""))
            }
        }
    }
}

/// A sealed `lines` task.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lines {
    /// The statement the participant reads: [`render`] of the topic and rules.
    pub statement: String,
    pub topic: String,
    pub rules: Vec<Rule>,
    pub salt: String,
}

/// What the generator writes for one task.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Candidate {
    pub topic: String,
    pub text: String,
}

/// A text as the rules see it.
struct Text {
    lines: Vec<Line>,
}

struct Line {
    raw: String,
    /// Lowercase.
    words: Vec<String>,
    first_letter: Option<char>,
}

impl Text {
    /// A line is a non-empty line of text. A word is a run of the letters
    /// A–Z and a–z with apostrophes allowed inside it (`'` or `’`); anything
    /// else separates words.
    fn read(text: &str) -> Text {
        let lines = text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(|raw| Line { raw: raw.to_string(), words: words(raw), first_letter: raw.chars().find(|c| c.is_ascii_alphabetic()) })
            .collect();
        Text { lines }
    }

    fn line(&self, number: u8) -> Result<&Line, String> {
        self.lines.get((number as usize).wrapping_sub(1)).ok_or_else(|| format!("the text has no line {number}"))
    }
}

fn words(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = line.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        let apostrophe = c == '\'' || c == '’';
        let inside = apostrophe
            && !current.is_empty()
            && chars.get(i + 1).is_some_and(|n| n.is_ascii_alphabetic());
        if c.is_ascii_alphabetic() {
            current.push(c.to_ascii_lowercase());
        } else if inside {
            current.push('\'');
        } else if !current.is_empty() {
            out.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// The statement for a topic and rules: fixed text, filled in.
pub fn render(topic: &str, rules: &[Rule]) -> String {
    let mut out = format!("Write a short text about {topic}.\n\nRules:\n");
    for (i, rule) in rules.iter().enumerate() {
        out.push_str(&format!("{}. {}\n", i + 1, rule.render()));
    }
    out.push_str(
        "\nDefinitions: a line is a non-empty line of text; empty lines do not count. A word is a run of the letters \
         A-Z and a-z, with apostrophes allowed inside it; digits, hyphens and all other characters separate words. \
         The first letter of a line is the first letter A-Z or a-z on it.",
    );
    out
}

impl Lines {
    pub fn validate(&self) -> Result<(), String> {
        let topic_ok = (3..=80).contains(&self.topic.len())
            && !self.topic.contains(['\n', '<', '>', '"'])
            && self.topic.chars().all(|c| c.is_ascii_graphic() || c == ' ');
        if !topic_ok {
            return Err("the topic must be 3 to 80 printable characters, with no quotes, tags or line breaks".to_string());
        }
        let n = match self.rules.first() {
            Some(Rule::LineCount { n }) => *n as usize,
            _ => return Err("the first rule is the line count".to_string()),
        };
        if !(MIN_LINES..=MAX_LINES).contains(&n) {
            return Err(format!("{n} lines; a task has {MIN_LINES} to {MAX_LINES}"));
        }
        match self.rules.get(1) {
            Some(Rule::Acrostic { letters }) if letters.len() == n && letters.bytes().all(|b| b.is_ascii_uppercase()) => {}
            _ => return Err("the second rule is an acrostic of one capital letter per line".to_string()),
        }
        if self.rules.len() != 2 + EXTRA_RULES {
            return Err(format!("{} rules; a task has {}", self.rules.len(), 2 + EXTRA_RULES));
        }
        for rule in &self.rules[2..] {
            match rule {
                Rule::WordsInLine { line, n: words } if (1..=n as u8).contains(line) && (1..=30).contains(words) => {}
                Rule::NoLetter { line, letter }
                    if (1..=n as u8).contains(line) && letter.len() == 1 && COMMON_LETTERS.contains(&letter.as_bytes()[0]) => {}
                Rule::WordCount { word, n: count } if is_word(word) && (1..=10).contains(count) => {}
                Rule::LastWord { word } if is_word(word) => {}
                other => return Err(format!("the rule `{}` is out of bounds", other.render())),
            }
        }
        if self.statement != render(&self.topic, &self.rules) {
            return Err("the statement is not the one its rules render to".to_string());
        }
        check_salt(&self.salt)
    }

    pub fn prompt(&self) -> String {
        format!("{}\n\nPut only the text between <answer> and </answer>, each line on its own line.", self.statement)
    }

    pub fn check(&self, answer: &str) -> Verdict {
        let text = Text::read(answer);
        for rule in &self.rules {
            if let Err(why) = rule.check(&text) {
                return Verdict::reject(why);
            }
        }
        Verdict::accept()
    }
}

fn is_word(w: &str) -> bool {
    !w.is_empty() && w.len() <= 20 && w.bytes().all(|b| b.is_ascii_lowercase() || b == b'\'') && w.as_bytes()[0].is_ascii_lowercase()
}

/// Build a task from the generator's topic and text, then run every
/// admission rule on it. `rng` makes the choices of which rules to draw; they
/// are recorded in the task.
pub fn admit(candidate: Candidate, rng: &mut Rng, salt: String) -> Result<Lines, Rejection> {
    let topic = candidate.topic.trim().trim_end_matches('.').to_string();
    let text = Text::read(&candidate.text);
    let n = text.lines.len();
    if !(MIN_LINES..=MAX_LINES).contains(&n) {
        return Err(Rejection::Malformed(format!("the text has {n} lines; a task has {MIN_LINES} to {MAX_LINES}")));
    }
    if text.lines.iter().any(|l| l.words.len() < 3 || l.first_letter.is_none()) {
        return Err(Rejection::Malformed("every line needs at least three words".to_string()));
    }
    let letters: String = text.lines.iter().map(|l| l.first_letter.unwrap().to_ascii_uppercase()).collect();
    let mut rules = vec![Rule::LineCount { n: n as u8 }, Rule::Acrostic { letters }];

    // Every extra rule the text can carry, in a fixed order; three are drawn.
    let mut available = Vec::new();
    let k = rng.below(n);
    available.push(Rule::WordsInLine { line: k as u8 + 1, n: text.lines[k].words.len().min(255) as u8 });
    let start = rng.below(n);
    if let Some((line, letter)) = (0..n).map(|i| (start + i) % n).find_map(|i| {
        COMMON_LETTERS.iter().find(|&&c| !text.lines[i].raw.chars().any(|x| x.eq_ignore_ascii_case(&(c as char)))).map(|&c| (i, c))
    }) {
        available.push(Rule::NoLetter { line: line as u8 + 1, letter: (letter as char).to_string() });
    }
    let all_words: Vec<&String> = text.lines.iter().flat_map(|l| &l.words).collect();
    let long: Vec<&String> = all_words.iter().copied().filter(|w| w.len() >= 5 && is_word(w)).collect();
    if !long.is_empty() {
        let word = long[rng.below(long.len())].clone();
        let count = all_words.iter().filter(|w| ***w == word).count().min(255) as u8;
        available.push(Rule::WordCount { word, n: count });
    }
    if let Some(last) = all_words.last().filter(|w| w.len() >= 3 && is_word(w)) {
        available.push(Rule::LastWord { word: (*last).clone() });
    }
    if available.len() < EXTRA_RULES {
        return Err(Rejection::Malformed("the text cannot carry enough rules".to_string()));
    }
    let mut drawn = rng.pick(available.len(), EXTRA_RULES);
    drawn.sort_unstable();
    rules.extend(drawn.into_iter().map(|i| available[i].clone()));

    let statement = render(&topic, &rules);
    let task = Lines { statement, topic, rules, salt };
    task.validate().map_err(Rejection::Malformed)?;

    let verdict = task.check(&candidate.text);
    if !verdict.ok {
        return Err(Rejection::WitnessRefused(verdict.reason.unwrap_or_default()));
    }
    let kept: Vec<&str> = candidate.text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let dropped = kept[..kept.len() - 1].join("\n");
    let reversed = kept.iter().rev().copied().collect::<Vec<_>>().join("\n");
    for (name, mutant) in [("the text with its last line dropped", dropped), ("the text with its lines reversed", reversed)] {
        if task.check(&mutant).ok {
            return Err(Rejection::MutationAccepted(name.to_string()));
        }
    }
    Ok(task)
}

pub fn parse_candidates(text: &str) -> Result<Vec<serde_json::Value>, String> {
    #[derive(Deserialize)]
    struct Batch {
        tasks: Vec<serde_json::Value>,
    }
    let json = json_object(text).ok_or("the generator wrote no JSON object")?;
    let batch: Batch = serde_json::from_str(json).map_err(|e| format!("the generator's JSON does not read: {e}"))?;
    Ok(batch.tasks)
}

pub fn generator_prompt(count: usize) -> String {
    format!(
        "Write {count} short texts, each on a different, specific topic of your choice (for example \"a lighthouse \
         keeper's first winter\" or \"how glass is recycled\").\n\n\
         Each text has {MIN_LINES} to {MAX_LINES} lines, one sentence per line, 6 to 14 words per line. Use plain \
         words, no digits, no hyphenated words, and vary how the lines begin.\n\n\
         Answer with one JSON object and nothing else:\n\
         {{\"tasks\": [{{\"topic\": \"...\", \"text\": \"line one\\nline two\\n...\"}}]}}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const SALT: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";
    const TEXT: &str = "Morning fog rolls over the quiet harbour.\n\
                        Old boats rock gently against the stone pier.\n\
                        Gulls circle above, calling for scraps.\n\
                        Nobody hurries here before the sun climbs high.\n\
                        Salt hangs in the air like a promise.";

    fn candidate() -> Candidate {
        Candidate { topic: "a harbour at dawn".into(), text: TEXT.into() }
    }

    #[test]
    fn words_and_lines_follow_the_statements_definitions() {
        assert_eq!(words("Don't stop—it's well-known, 42 times!"), ["don't", "stop", "it's", "well", "known", "times"]);
        assert_eq!(words("’Tis the season’s end'"), ["tis", "the", "season's", "end"]);
        let text = Text::read("\n  First line here  \n\n second line\n");
        assert_eq!(text.lines.len(), 2, "empty lines do not count");
        assert_eq!(text.lines[0].first_letter, Some('F'));
        assert_eq!(Text::read("— 1. alpha beta").lines[0].first_letter, Some('a'));
    }

    #[test]
    fn rules_are_read_off_the_authors_text_and_it_passes_them() {
        for seed in 0..50 {
            let task = admit(candidate(), &mut Rng::new(seed), SALT.into()).unwrap();
            assert_eq!(task.rules[0], Rule::LineCount { n: 5 });
            assert_eq!(task.rules[1], Rule::Acrostic { letters: "MOGNS".into() });
            assert_eq!(task.rules.len(), 5);
            assert!(task.check(TEXT).ok, "seed {seed}");
            assert!(task.validate().is_ok());
        }
    }

    #[test]
    fn the_checker_refuses_each_kind_of_miss() {
        let task = Lines {
            statement: String::new(),
            topic: "a harbour at dawn".into(),
            rules: vec![
                Rule::LineCount { n: 5 },
                Rule::Acrostic { letters: "MOGNS".into() },
                Rule::WordsInLine { line: 2, n: 8 },
                Rule::NoLetter { line: 3, letter: "t".into() },
                Rule::LastWord { word: "promise".into() },
            ],
            salt: SALT.into(),
        };
        assert!(task.check(TEXT).ok);
        let miss = |text: &str, part: &str| {
            let v = task.check(text);
            assert!(!v.ok, "{text}");
            assert!(v.reason.as_deref().unwrap().contains(part), "{:?} should mention {part}", v.reason);
        };
        miss(&TEXT.replace("Old boats", "Ancient boats"), "begin with");
        miss(&TEXT.replace("gently ", ""), "line 2 has 7 words");
        miss(&TEXT.replace("calling for", "calling to"), "contains the letter \"t\"");
        miss(&TEXT.replace("a promise.", "a vow."), "last word");
        miss(&format!("{TEXT}\nOne more line here."), "6 lines");
    }

    #[test]
    fn a_word_counts_whole_and_in_any_case() {
        let rule = Rule::WordCount { word: "the".into(), n: 3 };
        assert!(rule.check(&Text::read("The cat\nthe hat\nTHE bat, then")).is_ok(), "`then` is not `the`");
    }

    #[test]
    fn a_statement_that_is_not_its_rules_rendered_is_refused() {
        let mut task = admit(candidate(), &mut Rng::new(7), SALT.into()).unwrap();
        task.statement = task.statement.replace("exactly", "about");
        assert!(task.validate().unwrap_err().contains("not the one its rules render to"));
    }

    #[test]
    fn a_text_that_cannot_carry_a_task_is_not_admitted() {
        let short = Candidate { topic: "fog".into(), text: "One two three.\nFour five six.".into() };
        assert!(matches!(admit(short, &mut Rng::new(1), SALT.into()), Err(Rejection::Malformed(_))));
        let thin = Candidate { topic: "fog".into(), text: "Go.\nNo.\nSo.\nLo.".into() };
        assert!(matches!(admit(thin, &mut Rng::new(1), SALT.into()), Err(Rejection::Malformed(_))));
    }

    #[test]
    fn a_palindromic_acrostic_does_not_let_reversed_lines_through() {
        // First letters A, B, B, A: reversed lines keep the acrostic, but the
        // line count, word counts or last word still tell them apart — and
        // if nothing does, the task is refused.
        let text = "Ants march along the dusty path.\nBees hum softly in the warm clover field.\nBirds sing their evening song.\nA calm evening settles over everything now.";
        for seed in 0..50 {
            match admit(Candidate { topic: "a summer evening".into(), text: text.into() }, &mut Rng::new(seed), SALT.into()) {
                Ok(task) => assert!(!task.check(&text.lines().rev().collect::<Vec<_>>().join("\n")).ok),
                Err(e) => assert_eq!(e.code(), "mutation_accepted"),
            }
        }
    }
}

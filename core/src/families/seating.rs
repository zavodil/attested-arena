//! `seating`: who sits where in one row of seats.
//!
//! Code designs the puzzle inside the enclave: a random arrangement, then
//! conditions true of it, added until exactly one arrangement satisfies them
//! all and pruned while it still does ([`design`]). A language model writes
//! the story that states those conditions in plain English, and a second
//! model reads the story back into conditions. So the answer is right by
//! construction, and the only thing a model contributes — the wording — is
//! checked independently. Code then decides everything:
//!
//! | rule | refuses a task when |
//! |---|---|
//! | validator | sizes, names or seat numbers are out of bounds, or a condition repeats |
//! | witness | the author's own arrangement breaks a condition |
//! | solutions | no arrangement, or more than [`MAX_SOLUTIONS`], satisfies every condition (counted by brute force over all `n!` orders) |
//! | baselines | the people in the order they are listed, that order reversed, or the order the story first names them is a valid answer — the story would give the answer away |
//! | statement | a second model, given only the story, writes down different conditions |
//!
//! The checker accepts any arrangement that meets every condition, so a task
//! with two solutions is still fair.

use super::{json_object, Rejection};
use crate::{Rng, Verdict};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};

pub const MIN_PEOPLE: usize = 5;
pub const MAX_PEOPLE: usize = 8;
pub const MIN_CONDITIONS: usize = 4;
pub const MAX_CONDITIONS: usize = 14;
/// The most arrangements a task may allow.
pub const MAX_SOLUTIONS: usize = 2;
pub const MAX_STATEMENT: usize = 3000;

/// One condition. Seats are numbered 1 to n from left to right.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Condition {
    /// `a` sits somewhere to the left of `b`.
    LeftOf { a: String, b: String },
    /// `a` and `b` sit in neighbouring seats.
    Adjacent { a: String, b: String },
    /// `a` and `b` do not sit in neighbouring seats.
    NotAdjacent { a: String, b: String },
    /// `a` sits in seat `seat`.
    Seat { a: String, seat: u8 },
    /// `a` does not sit in seat `seat`.
    NotSeat { a: String, seat: u8 },
    /// `a` sits somewhere between `b` and `c`.
    Between { a: String, b: String, c: String },
    /// `a` sits in the first or the last seat.
    AtEnd { a: String },
}

impl Condition {
    fn names(&self) -> Vec<&str> {
        match self {
            Condition::LeftOf { a, b } | Condition::Adjacent { a, b } | Condition::NotAdjacent { a, b } => vec![a, b],
            Condition::Seat { a, .. } | Condition::NotSeat { a, .. } | Condition::AtEnd { a } => vec![a],
            Condition::Between { a, b, c } => vec![a, b, c],
        }
    }

    /// The same condition, written one way only: names lowercased, the
    /// symmetric pairs ordered.
    fn canonical(&self) -> Condition {
        let l = |s: &String| s.to_lowercase();
        match self {
            Condition::LeftOf { a, b } => Condition::LeftOf { a: l(a), b: l(b) },
            Condition::Adjacent { a, b } => {
                let (x, y) = ordered(l(a), l(b));
                Condition::Adjacent { a: x, b: y }
            }
            Condition::NotAdjacent { a, b } => {
                let (x, y) = ordered(l(a), l(b));
                Condition::NotAdjacent { a: x, b: y }
            }
            Condition::Seat { a, seat } => Condition::Seat { a: l(a), seat: *seat },
            Condition::NotSeat { a, seat } => Condition::NotSeat { a: l(a), seat: *seat },
            Condition::Between { a, b, c } => {
                let (x, y) = ordered(l(b), l(c));
                Condition::Between { a: l(a), b: x, c: y }
            }
            Condition::AtEnd { a } => Condition::AtEnd { a: l(a) },
        }
    }

    /// Whether the condition holds; `seat_of` gives each person's seat, 1-based.
    fn holds(&self, seat_of: &dyn Fn(&str) -> usize, n: usize) -> bool {
        match self {
            Condition::LeftOf { a, b } => seat_of(a) < seat_of(b),
            Condition::Adjacent { a, b } => seat_of(a).abs_diff(seat_of(b)) == 1,
            Condition::NotAdjacent { a, b } => seat_of(a).abs_diff(seat_of(b)) != 1,
            Condition::Seat { a, seat } => seat_of(a) == *seat as usize,
            Condition::NotSeat { a, seat } => seat_of(a) != *seat as usize,
            Condition::Between { a, b, c } => {
                let (lo, hi) = (seat_of(b).min(seat_of(c)), seat_of(b).max(seat_of(c)));
                lo < seat_of(a) && seat_of(a) < hi
            }
            Condition::AtEnd { a } => seat_of(a) == 1 || seat_of(a) == n,
        }
    }

    fn describe(&self) -> String {
        match self {
            Condition::LeftOf { a, b } => format!("{a} sits to the left of {b}"),
            Condition::Adjacent { a, b } => format!("{a} sits next to {b}"),
            Condition::NotAdjacent { a, b } => format!("{a} does not sit next to {b}"),
            Condition::Seat { a, seat } => format!("{a} sits in seat {seat}"),
            Condition::NotSeat { a, seat } => format!("{a} does not sit in seat {seat}"),
            Condition::Between { a, b, c } => format!("{a} sits between {b} and {c}"),
            Condition::AtEnd { a } => format!("{a} sits at an end of the row"),
        }
    }
}

fn ordered(a: String, b: String) -> (String, String) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

/// A sealed `seating` task.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Seating {
    /// The story the participant reads.
    pub statement: String,
    pub people: Vec<String>,
    pub conditions: Vec<Condition>,
    /// 32 random bytes, hex: makes the task's leaf unguessable.
    pub salt: String,
}

/// What the generator writes for one task.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Candidate {
    pub story: String,
    pub people: Vec<String>,
    pub conditions: Vec<Condition>,
    /// The arrangement the author had in mind, seat 1 first.
    pub arrangement: Vec<String>,
}

/// What the translator writes: the conditions it reads in the story.
#[derive(Clone, Debug, Deserialize)]
pub struct Reading {
    pub people: Vec<String>,
    pub conditions: Vec<Condition>,
}

impl Seating {
    pub fn validate(&self) -> Result<(), String> {
        let n = self.people.len();
        if !(MIN_PEOPLE..=MAX_PEOPLE).contains(&n) {
            return Err(format!("{n} people; a task has {MIN_PEOPLE} to {MAX_PEOPLE}"));
        }
        let mut seen = BTreeSet::new();
        for name in &self.people {
            let well_formed = (2..=12).contains(&name.len())
                && name.chars().next().is_some_and(|c| c.is_ascii_uppercase())
                && name.chars().all(|c| c.is_ascii_alphabetic());
            if !well_formed {
                return Err(format!("`{name}` is not a name: 2 to 12 letters A–Z, capitalised"));
            }
            if !seen.insert(name.to_lowercase()) {
                return Err(format!("`{name}` is named twice"));
            }
        }
        let m = self.conditions.len();
        if !(MIN_CONDITIONS..=MAX_CONDITIONS).contains(&m) {
            return Err(format!("{m} conditions; a task has {MIN_CONDITIONS} to {MAX_CONDITIONS}"));
        }
        let mut canonical = BTreeSet::new();
        for condition in &self.conditions {
            let names = condition.names();
            for name in &names {
                if !self.people.iter().any(|p| p == name) {
                    return Err(format!("the condition `{}` names `{name}`, who is not one of the people", condition.describe()));
                }
            }
            let distinct: BTreeSet<_> = names.iter().collect();
            if distinct.len() != names.len() {
                return Err(format!("the condition `{}` names one person twice", condition.describe()));
            }
            if let Condition::Seat { seat, .. } | Condition::NotSeat { seat, .. } = condition {
                if *seat == 0 || *seat as usize > n {
                    return Err(format!("seat {seat} does not exist: seats are 1 to {n}"));
                }
            }
            if !canonical.insert(condition.canonical()) {
                return Err(format!("the condition `{}` is stated twice", condition.describe()));
            }
        }
        if self.statement.trim().is_empty() || self.statement.len() > MAX_STATEMENT {
            return Err(format!("the story must be 1 to {MAX_STATEMENT} bytes"));
        }
        if self.statement.contains("<answer>") || self.statement.contains("</answer>") {
            return Err("the story contains an answer tag".to_string());
        }
        check_salt(&self.salt)
    }

    pub fn prompt(&self) -> String {
        let n = self.people.len();
        format!(
            "{}\n\nThe seats are in one row, numbered 1 to {n} from left to right, and each person takes exactly one seat. \
             \"Next to\" means in neighbouring seats; \"between\" means somewhere between, not necessarily next to either; \
             \"to the left of\" means somewhere to the left, not necessarily next to.\n\n\
             Give the order of the people from seat 1 to seat {n} as a comma-separated list of their names, \
             between <answer> and </answer>. Any order that meets every condition is accepted.",
            self.statement.trim()
        )
    }

    pub fn check(&self, answer: &str) -> Verdict {
        let order = match self.parse(answer) {
            Ok(order) => order,
            Err(why) => return Verdict::reject(why),
        };
        let n = self.people.len();
        let seat: HashMap<&str, usize> = order.iter().enumerate().map(|(i, &p)| (self.people[p].as_str(), i + 1)).collect();
        let seat_of = |name: &str| seat[name];
        for condition in &self.conditions {
            if !condition.holds(&seat_of, n) {
                return Verdict::reject(format!("the condition `{}` does not hold", condition.describe()));
            }
        }
        Verdict::accept()
    }

    /// The answer as a permutation: indices into `people`, seat 1 first.
    fn parse(&self, answer: &str) -> Result<Vec<usize>, String> {
        let names: Vec<&str> = answer
            .split(',')
            .map(|s| s.trim().trim_matches(|c: char| !c.is_ascii_alphabetic()))
            .filter(|s| !s.is_empty())
            .collect();
        if names.len() != self.people.len() {
            return Err(format!("{} names given; there are {} seats", names.len(), self.people.len()));
        }
        let mut order = Vec::with_capacity(names.len());
        for name in names {
            let index = self
                .people
                .iter()
                .position(|p| p.eq_ignore_ascii_case(name))
                .ok_or_else(|| format!("`{name}` is not one of the people"))?;
            if order.contains(&index) {
                return Err(format!("`{name}` is seated twice"));
            }
            order.push(index);
        }
        Ok(order)
    }

    /// How many arrangements satisfy every condition, counted up to `cap`.
    pub fn count_solutions(&self, cap: usize) -> usize {
        let n = self.people.len();
        let mut perm: Vec<usize> = (0..n).collect();
        let mut count = 0;
        let mut seat_of_person = vec![0usize; n];
        let index: HashMap<&str, usize> = self.people.iter().enumerate().map(|(i, p)| (p.as_str(), i)).collect();
        let mut visit = |perm: &[usize]| -> bool {
            for (s, &p) in perm.iter().enumerate() {
                seat_of_person[p] = s + 1;
            }
            let seat_of = |name: &str| seat_of_person[index[name]];
            if self.conditions.iter().all(|c| c.holds(&seat_of, n)) {
                count += 1;
            }
            count < cap
        };
        heap_permutations(&mut perm, &mut visit);
        count
    }

    /// The order in which the story first names each person.
    fn mention_order(&self) -> Result<Vec<String>, String> {
        let lower = self.statement.to_lowercase();
        let mut found = Vec::new();
        for person in &self.people {
            let at = find_word(&lower, &person.to_lowercase()).ok_or_else(|| format!("the story never names {person}"))?;
            found.push((at, person.clone()));
        }
        found.sort();
        Ok(found.into_iter().map(|(_, p)| p).collect())
    }

    fn canonical(people: &[String], conditions: &[Condition]) -> (BTreeSet<String>, BTreeSet<Condition>) {
        (people.iter().map(|p| p.to_lowercase()).collect(), conditions.iter().map(Condition::canonical).collect())
    }
}

/// Run a generated task through every admission rule.
pub fn admit(candidate: Candidate, reading: Option<Reading>, salt: String) -> Result<Seating, Rejection> {
    let task = Seating { statement: candidate.story, people: candidate.people, conditions: candidate.conditions, salt };
    task.validate().map_err(Rejection::Malformed)?;

    let witness = candidate.arrangement.join(", ");
    let verdict = task.check(&witness);
    if !verdict.ok {
        return Err(Rejection::WitnessRefused(verdict.reason.unwrap_or_default()));
    }
    match task.count_solutions(MAX_SOLUTIONS + 1) {
        0 => return Err(Rejection::Unsolvable),
        n if n > MAX_SOLUTIONS => return Err(Rejection::TooManySolutions(n)),
        _ => {}
    }

    let listed = task.people.clone();
    let mut reversed = listed.clone();
    reversed.reverse();
    let mentioned = task.mention_order().map_err(Rejection::Malformed)?;
    for (name, order) in [("the order the people are listed in", listed), ("that order reversed", reversed), ("the order the story names them in", mentioned)] {
        if task.check(&order.join(", ")).ok {
            return Err(Rejection::BaselineAccepted(name.to_string()));
        }
    }

    let reading = reading.ok_or_else(|| Rejection::StatementMismatch("no independent reading of the story".to_string()))?;
    let (people, conditions) = Seating::canonical(&task.people, &task.conditions);
    let (read_people, read_conditions) = Seating::canonical(&reading.people, &reading.conditions);
    if people != read_people {
        return Err(Rejection::StatementMismatch("the reader found other people".to_string()));
    }
    if conditions != read_conditions {
        let missing = conditions.difference(&read_conditions).count();
        let extra = read_conditions.difference(&conditions).count();
        return Err(Rejection::StatementMismatch(format!("{missing} conditions not read, {extra} read that the author did not state")));
    }
    Ok(task)
}

pub fn parse_reading(text: &str) -> Result<Reading, String> {
    let json = json_object(text).ok_or("the reader wrote no JSON object")?;
    serde_json::from_str(json).map_err(|e| format!("the reader's JSON does not read: {e}"))
}

const CONDITION_TYPES: &str = "\
- {\"type\": \"left_of\", \"a\": A, \"b\": B} — A sits somewhere to the left of B (not necessarily next to B)
- {\"type\": \"adjacent\", \"a\": A, \"b\": B} — A and B sit in neighbouring seats
- {\"type\": \"not_adjacent\", \"a\": A, \"b\": B} — A and B do not sit in neighbouring seats
- {\"type\": \"seat\", \"a\": A, \"seat\": K} — A sits in seat K
- {\"type\": \"not_seat\", \"a\": A, \"seat\": K} — A does not sit in seat K
- {\"type\": \"between\", \"a\": A, \"b\": B, \"c\": C} — A sits somewhere between B and C (not necessarily next to either)
- {\"type\": \"at_end\", \"a\": A} — A sits in the first or the last seat";

/// A puzzle as code designs it, before any story is written.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Plan {
    pub people: Vec<String>,
    pub conditions: Vec<Condition>,
    /// The only arrangement that satisfies every condition, seat 1 first.
    pub arrangement: Vec<String>,
}

const NAMES: &[&str] = &[
    "Ada", "Ben", "Cleo", "Dara", "Eli", "Faye", "Gus", "Hana", "Ivo", "Jun", "Kai", "Lena", "Milo", "Nora", "Omar",
    "Pia", "Quin", "Rosa", "Sami", "Tess", "Uma", "Vik", "Wren", "Yara", "Zane", "Arlo", "Bea", "Cyrus", "Dina",
    "Ezra", "Fern", "Gail", "Hugo", "Iris", "Jude", "Kira", "Liam", "Maya", "Nico", "Olga", "Petra", "Rhea", "Soren",
    "Theo", "Vera", "Wade", "Xena", "Yuri", "Zora", "Anya", "Boris", "Celia", "Dmitri", "Elsa", "Felix", "Greta",
];

fn count(people: &[String], conditions: &[Condition], cap: usize) -> usize {
    Seating { statement: String::new(), people: people.to_vec(), conditions: conditions.to_vec(), salt: String::new() }.count_solutions(cap)
}

/// A condition chosen at random among those true of `arrangement`. Ordering
/// conditions (between, left of, not next to) are drawn most often: they only
/// pin seats in combination, which is what makes a puzzle hard.
fn true_condition(rng: &mut Rng, arrangement: &[String]) -> Condition {
    let n = arrangement.len();
    let p = |i: usize| arrangement[i].clone();
    loop {
        match rng.below(13) {
            0..=3 => {
                let mut s = rng.pick(n, 3);
                s.sort_unstable();
                let (b, c) = if rng.below(2) == 0 { (p(s[0]), p(s[2])) } else { (p(s[2]), p(s[0])) };
                return Condition::Between { a: p(s[1]), b, c };
            }
            4..=6 => {
                let mut s = rng.pick(n, 2);
                s.sort_unstable();
                return Condition::LeftOf { a: p(s[0]), b: p(s[1]) };
            }
            7..=8 => {
                let s = rng.pick(n, 2);
                if s[0].abs_diff(s[1]) > 1 {
                    return Condition::NotAdjacent { a: p(s[0]), b: p(s[1]) };
                }
            }
            9..=10 => {
                let i = rng.below(n - 1);
                let (a, b) = if rng.below(2) == 0 { (p(i), p(i + 1)) } else { (p(i + 1), p(i)) };
                return Condition::Adjacent { a, b };
            }
            11 => {
                let i = rng.below(n);
                let mut seat = rng.below(n);
                if seat == i {
                    seat = (seat + 1) % n;
                }
                return Condition::NotSeat { a: p(i), seat: seat as u8 + 1 };
            }
            _ => {
                let i = if rng.below(2) == 0 { 0 } else { n - 1 };
                return Condition::AtEnd { a: p(i) };
            }
        }
    }
}

/// Design one puzzle of `n` people: conditions true of a random arrangement
/// are added until it is the only solution, then each is dropped if the
/// solution stays unique without it.
pub fn design(rng: &mut Rng, n: usize) -> Plan {
    let n = n.clamp(MIN_PEOPLE, MAX_PEOPLE);
    let people: Vec<String> = rng.pick(NAMES.len(), n).into_iter().map(|i| NAMES[i].to_string()).collect();
    loop {
        let arrangement: Vec<String> = rng.pick(n, n).into_iter().map(|i| people[i].clone()).collect();
        let mut conditions: Vec<Condition> = Vec::new();
        let mut seen = BTreeSet::new();
        while conditions.len() < 40 && count(&people, &conditions, 2) > 1 {
            let c = true_condition(rng, &arrangement);
            if seen.insert(c.canonical()) {
                conditions.push(c);
            }
        }
        if count(&people, &conditions, 2) != 1 {
            continue;
        }
        // Drop any condition the answer does not need, until none can go.
        'prune: while conditions.len() > MIN_CONDITIONS {
            for i in rng.pick(conditions.len(), conditions.len()) {
                let mut fewer = conditions.clone();
                fewer.remove(i);
                if count(&people, &fewer, 2) == 1 {
                    conditions = fewer;
                    continue 'prune;
                }
            }
            break;
        }
        if (MIN_CONDITIONS..=MAX_CONDITIONS).contains(&conditions.len()) {
            return Plan { people, conditions, arrangement };
        }
    }
}

/// The prompt the story writer gets: the puzzles as data, to be told in
/// plain English, one story each, in order.
pub fn story_prompt(plans: &[Plan]) -> String {
    let puzzles: Vec<serde_json::Value> = plans
        .iter()
        .map(|p| serde_json::json!({ "people": p.people, "conditions": p.conditions }))
        .collect();
    format!(
        "Each puzzle below is about people sitting in one row of seats, numbered 1 to n from left to right, given as \
         data. For each puzzle, write a short story (3 to 8 sentences, a setting of your choice: a cinema, a bus, a \
         dinner, a photo...) that states EVERY condition in plain English and states nothing else about the seats.\n\n\
         Condition types:\n{CONDITION_TYPES}\n\n\
         Rules for the story: name every person; keep each condition's meaning exact (\"between\" is not \"next to\", \
         \"to the left of\" is not \"directly left of\"); do not add hints, do not solve the puzzle, and do not \
         mention the people in seat order.\n\n\
         Puzzles:\n{}\n\n\
         Answer with one JSON object and nothing else: {{\"stories\": [\"story for puzzle 1\", \"story for puzzle 2\", ...]}}",
        serde_json::to_string_pretty(&puzzles).expect("puzzles serialise")
    )
}

pub fn parse_stories(text: &str) -> Result<Vec<String>, String> {
    #[derive(Deserialize)]
    struct Stories {
        stories: Vec<String>,
    }
    let json = json_object(text).ok_or("the writer wrote no JSON object")?;
    let s: Stories = serde_json::from_str(json).map_err(|e| format!("the writer's JSON does not read: {e}"))?;
    Ok(s.stories)
}

impl Plan {
    /// The plan with its story, as admission takes it.
    pub fn with_story(self, story: String) -> Candidate {
        Candidate { story, people: self.people, conditions: self.conditions, arrangement: self.arrangement }
    }
}

/// The prompt the second model gets: the story alone, to be written back as
/// data. Its reading must match the author's conditions exactly.
pub fn reader_prompt(story: &str) -> String {
    format!(
        "Below is a puzzle about people sitting in one row of seats, numbered 1 to n from left to right. \
         Write down, as data, exactly the conditions the story states — no more, no fewer. Do not solve it.\n\n\
         Condition types:\n{CONDITION_TYPES}\n\n\
         Story:\n{story}\n\n\
         Answer with one JSON object and nothing else: {{\"people\": [\"...\"], \"conditions\": [...]}}"
    )
}

/// The `k`-th position (byte index) where `word` occurs in `text` as a whole
/// word; `text` and `word` are lowercase.
fn find_word(text: &str, word: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut from = 0;
    while let Some(i) = text[from..].find(word) {
        let at = from + i;
        let end = at + word.len();
        let before = at == 0 || !bytes[at - 1].is_ascii_alphabetic();
        let after = end == bytes.len() || !bytes[end].is_ascii_alphabetic();
        if before && after {
            return Some(at);
        }
        from = at + 1;
    }
    None
}

/// Every permutation of `items` (Heap's algorithm), until `visit` says stop.
fn heap_permutations(items: &mut [usize], visit: &mut dyn FnMut(&[usize]) -> bool) {
    let n = items.len();
    let mut c = vec![0usize; n];
    if !visit(items) {
        return;
    }
    let mut i = 0;
    while i < n {
        if c[i] < i {
            if i % 2 == 0 {
                items.swap(0, i);
            } else {
                items.swap(c[i], i);
            }
            if !visit(items) {
                return;
            }
            c[i] += 1;
            i = 0;
        } else {
            c[i] = 0;
            i += 1;
        }
    }
}

pub(crate) fn check_salt(salt: &str) -> Result<(), String> {
    if salt.len() == 64 && salt.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()) {
        Ok(())
    } else {
        Err("the salt is not 32 bytes of lowercase hex".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SALT: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

    fn c(json: &str) -> Condition {
        serde_json::from_str(json).unwrap()
    }

    /// Five friends; Ann, Ben, Cat, Dan, Eve sit in seats 1 to 5. The
    /// conditions pin that one arrangement.
    fn candidate() -> Candidate {
        Candidate {
            story: "At the cinema, Eve wanted the aisle seat at the far right. Cat sat in the middle seat. \
                    Ben sat next to Ann, and Dan sat somewhere between Cat and Eve. Ann sat to the left of Ben."
                .into(),
            people: ["Eve", "Cat", "Ben", "Ann", "Dan"].map(String::from).to_vec(),
            conditions: vec![
                c(r#"{"type":"seat","a":"Eve","seat":5}"#),
                c(r#"{"type":"seat","a":"Cat","seat":3}"#),
                c(r#"{"type":"adjacent","a":"Ben","b":"Ann"}"#),
                c(r#"{"type":"between","a":"Dan","b":"Cat","c":"Eve"}"#),
                c(r#"{"type":"left_of","a":"Ann","b":"Ben"}"#),
            ],
            arrangement: ["Ann", "Ben", "Cat", "Dan", "Eve"].map(String::from).to_vec(),
        }
    }

    fn reading_of(candidate: &Candidate) -> Reading {
        Reading { people: candidate.people.clone(), conditions: candidate.conditions.clone() }
    }

    #[test]
    fn a_sound_task_is_admitted_and_its_checker_decides_answers() {
        let cand = candidate();
        let task = admit(cand.clone(), Some(reading_of(&cand)), SALT.into()).unwrap();
        assert!(task.check("Ann, Ben, Cat, Dan, Eve").ok);
        assert!(task.check(" ann ,BEN, cat, dan, Eve. ").ok, "names are matched without regard to case or stray punctuation");
        let wrong = task.check("Ben, Ann, Cat, Eve, Dan");
        assert!(!wrong.ok);
        assert!(wrong.reason.unwrap().contains("does not hold"));
        assert!(!task.check("Ann, Ben, Cat, Dan").ok, "a seat left empty");
        assert!(!task.check("Ann, Ann, Cat, Dan, Eve").ok, "a person seated twice");
        assert!(!task.check("Ann, Ben, Cat, Dan, Zed").ok, "a stranger");
    }

    #[test]
    fn the_authors_own_solution_must_be_accepted() {
        let mut cand = candidate();
        cand.arrangement = ["Ben", "Ann", "Cat", "Dan", "Eve"].map(String::from).to_vec();
        let reading = reading_of(&cand);
        assert!(matches!(admit(cand, Some(reading), SALT.into()), Err(Rejection::WitnessRefused(_))));
    }

    #[test]
    fn a_task_with_too_many_answers_is_not_admitted() {
        let mut cand = candidate();
        // Ann at an end, Cat and Dan not in seat 1, Ann away from Eve: the
        // author's arrangement still fits, and so do many others.
        cand.conditions = vec![
            c(r#"{"type":"at_end","a":"Ann"}"#),
            c(r#"{"type":"not_seat","a":"Cat","seat":1}"#),
            c(r#"{"type":"not_seat","a":"Dan","seat":1}"#),
            c(r#"{"type":"not_adjacent","a":"Ann","b":"Eve"}"#),
        ];
        let reading = reading_of(&cand);
        assert!(matches!(admit(cand, Some(reading), SALT.into()), Err(Rejection::TooManySolutions(_))));
    }

    #[test]
    fn a_story_that_names_people_in_seat_order_gives_the_answer_away() {
        let mut cand = candidate();
        cand.story = "Ann, Ben, Cat, Dan and Eve went to the cinema. Eve took seat 5 and Cat seat 3. \
                      Ben sat next to Ann; Dan sat between Cat and Eve; Ann sat to the left of Ben."
            .into();
        let reading = reading_of(&cand);
        assert_eq!(
            admit(cand, Some(reading), SALT.into()),
            Err(Rejection::BaselineAccepted("the order the story names them in".into()))
        );
    }

    #[test]
    fn a_story_a_second_reader_understands_differently_is_not_admitted() {
        let cand = candidate();
        let mut reading = reading_of(&cand);
        // The reader took "Dan sat somewhere between Cat and Eve" for "next to Eve".
        reading.conditions[3] = c(r#"{"type":"adjacent","a":"Dan","b":"Eve"}"#);
        let err = admit(cand.clone(), Some(reading), SALT.into()).unwrap_err();
        assert_eq!(err.code(), "statement_mismatch");
        assert!(matches!(admit(cand, None, SALT.into()), Err(Rejection::StatementMismatch(_))));
    }

    #[test]
    fn a_reading_matches_up_to_case_and_the_order_of_symmetric_pairs() {
        let cand = candidate();
        let mut reading = reading_of(&cand);
        reading.conditions.reverse();
        reading.conditions[2] = c(r#"{"type":"adjacent","a":"ann","b":"BEN"}"#);
        reading.conditions[1] = c(r#"{"type":"between","a":"Dan","b":"Eve","c":"Cat"}"#);
        assert!(admit(cand, Some(reading), SALT.into()).is_ok());
    }

    #[test]
    fn the_validator_refuses_what_is_out_of_bounds() {
        let base = admit(candidate(), Some(reading_of(&candidate())), SALT.into()).unwrap();
        let mut t = base.clone();
        t.conditions.push(c(r#"{"type":"seat","a":"Ann","seat":9}"#));
        assert!(t.validate().unwrap_err().contains("seat 9"));
        let mut t = base.clone();
        t.conditions.push(c(r#"{"type":"left_of","a":"Ann","b":"Zed"}"#));
        assert!(t.validate().unwrap_err().contains("Zed"));
        let mut t = base.clone();
        t.conditions.push(c(r#"{"type":"adjacent","a":"Ann","b":"Ben"}"#));
        assert!(t.validate().unwrap_err().contains("twice"), "Ben next to Ann, stated the other way round, is the same condition");
        let mut t = base.clone();
        t.people.push("eve".into());
        assert!(t.validate().is_err());
        let mut t = base.clone();
        t.statement.push_str(" <answer>Ann</answer>");
        assert!(t.validate().is_err());
        let mut t = base;
        t.salt = "XYZ".into();
        assert!(t.validate().is_err());
    }

    #[test]
    fn solutions_are_counted_over_every_order() {
        let task = admit(candidate(), Some(reading_of(&candidate())), SALT.into()).unwrap();
        assert_eq!(task.count_solutions(10), 1);
        let free = Seating { conditions: vec![c(r#"{"type":"at_end","a":"Ann"}"#)], ..task };
        // Ann at either end, the other four in any order: 2 × 4! = 48.
        assert_eq!(free.count_solutions(1000), 48);
    }

    #[test]
    fn a_designed_puzzle_has_exactly_one_answer_and_needs_every_condition() {
        for seed in 0..30 {
            let mut rng = Rng::new(seed);
            let plan = design(&mut rng, 5 + (seed as usize % 4));
            let n = plan.people.len();
            assert_eq!(count(&plan.people, &plan.conditions, 3), 1, "seed {seed}");
            let task = Seating { statement: "x".into(), people: plan.people.clone(), conditions: plan.conditions.clone(), salt: SALT.into() };
            assert!(task.check(&plan.arrangement.join(", ")).ok, "the planted arrangement is the answer");
            assert!((MIN_CONDITIONS..=MAX_CONDITIONS).contains(&plan.conditions.len()));
            if plan.conditions.len() > MIN_CONDITIONS {
                for i in 0..plan.conditions.len() {
                    let mut fewer = plan.conditions.clone();
                    fewer.remove(i);
                    assert!(count(&plan.people, &fewer, 3) > 1, "seed {seed}: condition {i} is redundant");
                }
            }
            assert!((MIN_PEOPLE..=MAX_PEOPLE).contains(&n));
        }
    }

    #[test]
    fn heap_visits_every_permutation_once() {
        let mut seen = std::collections::HashSet::new();
        let mut items = vec![0, 1, 2, 3];
        heap_permutations(&mut items, &mut |p| {
            seen.insert(p.to_vec());
            true
        });
        assert_eq!(seen.len(), 24);
    }
}

//! How a duel is decided: a sequential probability ratio test on the tasks
//! where the two models disagree.
//!
//! Both models answer the same tasks. A task both solve, or both miss, says
//! nothing about which is better; only a discordant task does. Under H0 the
//! challenger wins a discordant task with probability p0 = 1/2 (it is no
//! better); under H1 with p1 > 1/2. After each task the log-likelihood ratio
//! moves by ln(p1/p0) for a challenger's win and ln((1−p1)/(1−p0)) for a
//! loss; the duel stops when it crosses ln((1−β)/α) (better) or ln(β/(1−α))
//! (not shown better). Chess engines are tested this way (Stockfish's
//! fishtest plays paired games and stops on SPRT).
//!
//! The contract adds integers: each step is the log ratio times 10^6,
//! rounded, computed here once and stored with the season.

use serde::{Deserialize, Serialize};

pub const SCALE: f64 = 1_000_000.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sprt {
    /// Added when only the challenger is right.
    pub win: i64,
    /// Added when only the champion is right (negative).
    pub loss: i64,
    /// The challenger is shown better at or above this.
    pub accept: i64,
    /// The challenger is not shown better at or below this (negative).
    pub reject: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Continue,
    Accept,
    Reject,
}

impl Sprt {
    pub fn new(p1: f64, alpha: f64, beta: f64) -> Sprt {
        assert!(p1 > 0.5 && p1 < 1.0 && alpha > 0.0 && alpha < 0.5 && beta > 0.0 && beta < 0.5);
        let fixed = |x: f64| (x * SCALE).round() as i64;
        Sprt {
            win: fixed((p1 / 0.5).ln()),
            loss: fixed(((1.0 - p1) / 0.5).ln()),
            accept: fixed(((1.0 - beta) / alpha).ln()),
            reject: fixed((beta / (1.0 - alpha)).ln()),
        }
    }

    /// The arena's parameters: H1 is "the challenger wins 70% of the tasks the
    /// two disagree on"; 5% false records, 10% missed ones, per phase.
    pub fn arena() -> Sprt {
        Sprt::new(0.7, 0.05, 0.10)
    }

    pub fn step(&self, llr: i64, champion_ok: bool, challenger_ok: bool) -> i64 {
        match (champion_ok, challenger_ok) {
            (false, true) => llr + self.win,
            (true, false) => llr + self.loss,
            _ => llr,
        }
    }

    pub fn decide(&self, llr: i64) -> Decision {
        if llr >= self.accept {
            Decision::Accept
        } else if llr <= self.reject {
            Decision::Reject
        } else {
            Decision::Continue
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_arena_parameters_are_these_integers() {
        assert_eq!(Sprt::arena(), Sprt { win: 336472, loss: -510826, accept: 2890372, reject: -2251292 });
    }

    #[test]
    fn agreement_moves_nothing_and_a_clear_winner_is_accepted() {
        let s = Sprt::arena();
        assert_eq!(s.step(0, true, true), 0);
        assert_eq!(s.step(0, false, false), 0);
        let mut llr = 0;
        let mut wins = 0;
        while s.decide(llr) == Decision::Continue {
            llr = s.step(llr, false, true);
            wins += 1;
        }
        assert_eq!((s.decide(llr), wins), (Decision::Accept, 9), "nine discordant wins in a row");
        let mut llr = 0;
        let mut losses = 0;
        while s.decide(llr) == Decision::Continue {
            llr = s.step(llr, true, false);
            losses += 1;
        }
        assert_eq!((s.decide(llr), losses), (Decision::Reject, 5));
    }

    /// The error rates hold: simulate many duels with the challenger at p0
    /// (no better) and at p1, and count the decisions.
    #[test]
    fn error_rates_are_what_the_parameters_say() {
        let s = Sprt::arena();
        let mut rng = crate::Rng::new(42);
        let run = |p: f64, rng: &mut crate::Rng| {
            let mut llr = 0;
            loop {
                let challenger_wins = (rng.next_u64() as f64 / u64::MAX as f64) < p;
                llr = s.step(llr, !challenger_wins, challenger_wins);
                match s.decide(llr) {
                    Decision::Continue => {}
                    d => return d,
                }
            }
        };
        let trials = 20_000;
        let false_records = (0..trials).filter(|_| run(0.5, &mut rng) == Decision::Accept).count();
        let missed = (0..trials).filter(|_| run(0.7, &mut rng) == Decision::Reject).count();
        assert!((false_records as f64 / trials as f64) < 0.05, "{false_records}");
        assert!((missed as f64 / trials as f64) < 0.10, "{missed}");
    }
}

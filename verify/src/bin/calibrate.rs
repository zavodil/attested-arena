//! A development tool: tune the families before a season, outside any TEE.
//!
//! It runs the same generation, admission and checking code the judge runs,
//! against NEAR AI directly from this machine, and prints how many generated
//! tasks were admitted and how many each model solves. Tasks made here are
//! seen by whoever runs it, so they never enter a season.
//!
//! ```text
//! NEARAI_API_KEY=… arena-calibrate <family> <count> <generator> <reader> <model>...
//! ```

use arena_core::answer;
use arena_core::families::{lines, seating, Rejection};
use arena_core::request;
use arena_core::task::Task;
use arena_core::Rng;
use std::collections::BTreeMap;

fn chat(http: &reqwest::blocking::Client, key: &str, body: Vec<u8>) -> Result<Vec<u8>, String> {
    let r = http
        .post("https://cloud-api.near.ai/v1/chat/completions")
        .header("Authorization", format!("Bearer {key}"))
        .header("Content-Type", "application/json")
        .timeout(std::time::Duration::from_secs(300))
        .body(body)
        .send()
        .map_err(|e| format!("transport: {}", e.without_url()))?;
    let status = r.status();
    let bytes = r.bytes().map_err(|e| e.without_url().to_string())?.to_vec();
    if !status.is_success() {
        return Err(format!("HTTP {status}: {}", String::from_utf8_lossy(&bytes).chars().take(300).collect::<String>()));
    }
    Ok(bytes)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 6 {
        eprintln!("usage: arena-calibrate <seating|lines> <count> <generator> <reader> <model>...");
        std::process::exit(2);
    }
    let key = std::env::var("NEARAI_API_KEY").expect("NEARAI_API_KEY");
    let (family, count, generator, reader) = (&args[1], args[2].parse::<usize>().unwrap(), &args[3], &args[4]);
    let models = &args[5..];
    let http = reqwest::blocking::Client::new();
    let started = std::time::Instant::now();

    let salt = "00".repeat(32);
    let mut rng = Rng::new(started.elapsed().as_nanos() as u64 ^ 0x5eed);
    let mut tasks = Vec::new();
    let mut rejected: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut reject = |r: Rejection| rejected.entry(r.code().to_string()).or_default().push(r.to_string());
    if family == "seating" {
        let plans: Vec<seating::Plan> = (0..count).map(|i| seating::design(&mut rng, 6 + i % 2)).collect();
        println!("designed {count} puzzles in {:.1}s", started.elapsed().as_secs_f64());
        let resp = chat(&http, &key, request::chat(generator, &seating::story_prompt(&plans), 8000)).expect("stories");
        println!("stories in {:.0}s", started.elapsed().as_secs_f64());
        let stories = seating::parse_stories(&answer::content(&resp).expect("content")).expect("stories json");
        for (plan, story) in plans.into_iter().zip(stories) {
            let c = plan.with_story(story);
            let read = chat(&http, &key, request::chat(reader, &seating::reader_prompt(&c.story), 3000))
                .ok()
                .and_then(|r| answer::content(&r).ok())
                .and_then(|t| seating::parse_reading(&t).ok());
            match seating::admit(c, read, salt.clone()) {
                Ok(t) => tasks.push(Task::Seating(t)),
                Err(r) => reject(r),
            }
        }
    } else {
        let resp = chat(&http, &key, request::chat(generator, &lines::generator_prompt(count), 8000)).expect("generation");
        println!("generated in {:.0}s", started.elapsed().as_secs_f64());
        for c in lines::parse_candidates(&answer::content(&resp).expect("content")).expect("candidates") {
            match serde_json::from_value::<lines::Candidate>(c) {
                Err(e) => reject(Rejection::Malformed(e.to_string())),
                Ok(c) => match lines::admit(c, &mut rng, salt.clone()) {
                    Ok(t) => tasks.push(Task::Lines(t)),
                    Err(r) => reject(r),
                },
            }
        }
    }
    println!("admitted {} — rejected {:?}", tasks.len(), rejected.iter().map(|(k, v)| (k, v.len())).collect::<Vec<_>>());
    for (k, v) in &rejected {
        for why in v.iter().take(3) {
            println!("   {k}: {}", why.chars().take(160).collect::<String>());
        }
    }
    if let Some(t) = tasks.first() {
        println!("\n--- example prompt ---\n{}\n---", t.prompt());
    }
    for m in models {
        let t0 = std::time::Instant::now();
        let mut ok = 0;
        let mut errors = 0;
        for t in &tasks {
            match chat(&http, &key, request::participant(m, t)) {
                Ok(r) => {
                    let g = answer::grade(t, &r);
                    if g.verdict.ok {
                        ok += 1;
                    } else if ok + errors < 2 {
                        println!("   {m} miss: {:?}", g.verdict.reason);
                    }
                }
                Err(e) => {
                    errors += 1;
                    println!("   {m} error: {e}");
                }
            }
        }
        println!("{m}: {ok}/{} right, {errors} errors, {:.0}s", tasks.len(), t0.elapsed().as_secs_f64());
    }
}

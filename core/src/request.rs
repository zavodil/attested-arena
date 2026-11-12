//! The exact request a participant model is sent.
//!
//! It is a function of the task, the model and nothing else, so anyone holding
//! a revealed task rebuilds the same bytes, hashes them, and checks that the
//! model's TEE signed an exchange over exactly that request.

use crate::sha256_hex;
use crate::task::Task;
use serde::Serialize;

/// The most a participant may write. Thinking is off: an answer is the
/// model's own text, not a hidden chain of thought.
pub const MAX_TOKENS: u32 = 1000;

#[derive(Serialize)]
struct Body<'a> {
    model: &'a str,
    messages: [Message<'a>; 1],
    temperature: u8,
    max_tokens: u32,
    chat_template_kwargs: Kwargs,
}

#[derive(Serialize)]
struct Message<'a> {
    role: &'static str,
    content: &'a str,
}

#[derive(Serialize)]
struct Kwargs {
    enable_thinking: bool,
}

/// `POST /v1/chat/completions` body for `model` on `task`.
pub fn participant(model: &str, task: &Task) -> Vec<u8> {
    chat(model, &task.prompt(), MAX_TOKENS)
}

/// A one-message chat at temperature 0 with thinking off.
pub fn chat(model: &str, prompt: &str, max_tokens: u32) -> Vec<u8> {
    let body = Body {
        model,
        messages: [Message { role: "user", content: prompt }],
        temperature: 0,
        max_tokens,
        chat_template_kwargs: Kwargs { enable_thinking: false },
    };
    serde_json::to_vec(&body).expect("a request always serialises")
}

/// The nonce a model's attestation is asked for in one run: sha256 of the
/// run's request hashes for that model, one per line, in task order. The
/// report is then fresh for exactly the requests it vouches for.
pub fn attestation_nonce(request_sha256s: &[String]) -> String {
    sha256_hex(request_sha256s.join("\n").as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_body_is_fixed_byte_for_byte() {
        assert_eq!(
            String::from_utf8(chat("m/x", "hi \"there\"", 10)).unwrap(),
            r#"{"model":"m/x","messages":[{"role":"user","content":"hi \"there\""}],"temperature":0,"max_tokens":10,"chat_template_kwargs":{"enable_thinking":false}}"#
        );
    }
}

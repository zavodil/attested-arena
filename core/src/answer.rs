//! Reading a participant's answer out of a chat completion.
//!
//! The answer is the text between the last `<answer>` and the `</answer>`
//! after it, in the completion's message content, after any `</think>`.
//! Nothing else the model wrote is scored.

use crate::task::Task;
use crate::Verdict;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Graded {
    /// The text between the tags; absent when the model gave none.
    pub answer: Option<String>,
    pub verdict: Verdict,
}

/// `choices[0].message.content` of a chat completion.
pub fn content(response_body: &[u8]) -> Result<String, String> {
    let value: serde_json::Value =
        serde_json::from_slice(response_body).map_err(|e| format!("the completion is not JSON: {e}"))?;
    value
        .pointer("/choices/0/message/content")
        .and_then(|c| c.as_str())
        .map(str::to_string)
        .ok_or_else(|| "the completion has no message content".to_string())
}

pub fn extract(content: &str) -> Option<String> {
    let visible = content.rsplit_once("</think>").map_or(content, |(_, after)| after);
    let start = visible.rfind("<answer>")? + "<answer>".len();
    let end = visible[start..].find("</answer>")? + start;
    Some(visible[start..end].trim().to_string())
}

/// Score one completion against one task.
pub fn grade(task: &Task, response_body: &[u8]) -> Graded {
    let answer = content(response_body).ok().and_then(|c| extract(&c));
    let verdict = match &answer {
        Some(a) => task.check(a),
        None => Verdict::reject("no <answer> block"),
    };
    Graded { answer, verdict }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_answer_after_thinking_is_the_one_scored() {
        assert_eq!(extract("<answer>a</answer> then <answer> b </answer>").as_deref(), Some("b"));
        assert_eq!(extract("<think><answer>x</answer></think>no tags"), None, "an answer inside the thinking does not count");
        assert_eq!(extract("<answer>open"), None);
        assert_eq!(extract("<answer>\nl1\nl2\n</answer>").as_deref(), Some("l1\nl2"));
    }

    #[test]
    fn content_is_read_from_the_first_choice() {
        let body = br#"{"choices":[{"message":{"role":"assistant","content":"<answer>Ann</answer>"}}]}"#;
        assert_eq!(content(body).unwrap(), "<answer>Ann</answer>");
        assert!(content(br#"{"error":"x"}"#).is_err());
    }
}

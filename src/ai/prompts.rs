//! Prompt templates for the AI-first email actions.
//!
//! Every template returns a `(system, user)` pair. The templates are pure
//! text construction (no network, no state) so they can be unit-tested and
//! tuned independently of any model.

/// Build a summarize prompt: 2-3 sentences, preserve facts and action items.
pub fn summarize(sender: Option<&str>, subject: Option<&str>, body: &str) -> (String, String) {
    let system = "You are Komorebi's private email assistant. Summarize the email below in 2-3 concise sentences. Preserve key facts, dates, decisions, and any requested action. Reply with plain text only and no preamble.".to_string();
    let user = format!(
        "From: {}\nSubject: {}\n\n{}",
        sender.unwrap_or("(unknown sender)"),
        subject.unwrap_or("(no subject)"),
        body.trim()
    );
    (system, user)
}

/// Build a draft-reply prompt. `tone` is freeform guidance (e.g. "friendly",
/// "brief and formal").
pub fn draft_reply(
    sender: Option<&str>,
    subject: Option<&str>,
    body: &str,
    tone: &str,
) -> (String, String) {
    let system = format!(
        "You are Komorebi's private email assistant. Draft a reply to the email below. Tone: {}. Write only the reply body: no subject line, no greeting line that duplicates the recipient's name is required, and no signature block. Plain text only.",
        tone.trim()
    );
    let user = format!(
        "Original email  - \nFrom: {}\nSubject: {}\n\n{}",
        sender.unwrap_or("(unknown sender)"),
        subject.unwrap_or("(no subject)"),
        body.trim()
    );
    (system, user)
}

/// Build a polish prompt for a draft the user wrote.
pub fn polish(draft: &str, tone: &str) -> (String, String) {
    let system = format!(
        "You are Komorebi's private email assistant. Improve the draft below for clarity and correctness while preserving its meaning and the user's voice. Tone: {}. Return only the improved draft, with no preamble or explanation.",
        tone.trim()
    );
    let user = format!("Draft:\n\n{}", draft.trim());
    (system, user)
}

/// Build a triage prompt: classify into exactly one category.
pub fn triage(sender: Option<&str>, subject: Option<&str>, body: &str) -> (String, String) {
    let system = "You are Komorebi's private email assistant. Classify the email below into exactly one category: work, personal, finance, newsletters, or updates. Reply with only the single lowercase word and nothing else.".to_string();
    let user = format!(
        "From: {}\nSubject: {}\n\n{}",
        sender.unwrap_or("(unknown sender)"),
        subject.unwrap_or("(no subject)"),
        body.trim()
    );
    (system, user)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarize_includes_envelope_and_body() {
        let (system, user) = summarize(Some("Jane"), Some("Report"), "The report is ready.");
        assert!(system.contains("Summarize"));
        assert!(user.contains("From: Jane"));
        assert!(user.contains("Subject: Report"));
        assert!(user.contains("The report is ready."));
    }

    #[test]
    fn draft_reply_carries_tone() {
        let (system, user) = draft_reply(
            Some("Jane"),
            Some("Hi"),
            "Can you help?",
            "brief and formal",
        );
        assert!(system.contains("brief and formal"));
        assert!(user.contains("Can you help?"));
    }

    #[test]
    fn triage_asks_for_single_word() {
        let (system, _) = triage(None, None, "body");
        assert!(system.contains("exactly one category"));
        assert!(system.contains("single lowercase word"));
    }

    #[test]
    fn polish_returns_only_improved_draft() {
        let (system, user) = polish("Helo team,\nmeeting moved", "professional");
        assert!(system.contains("no preamble"));
        assert!(user.contains("Helo team"));
    }
}

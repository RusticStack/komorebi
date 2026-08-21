//! Prompt templates for the AI-first email actions.
//!
//! Every template returns a `(system, user)` pair. The templates are pure
//! text construction (no network, no state) so they can be unit-tested and
//! tuned independently of any model.

/// Build a summarize prompt: 2-3 sentences, preserve facts and action items.
pub fn summarize(sender: Option<&str>, subject: Option<&str>, body: &str) -> (String, String) {
    let system = "You are Komorebi's private email assistant. Summarize the email below in 2-3 concise sentences. Preserve key facts, dates, decisions, and any requested action. If you are uncertain about a fact, say so explicitly instead of inventing details. Reply with plain text only and no preamble.".to_string();
    let user = format!(
        "From: {}\nSubject: {}\n\n{}",
        sender.unwrap_or("(unknown sender)"),
        subject.unwrap_or("(no subject)"),
        body.trim()
    );
    (system, user)
}

/// Build a summarize prompt for an entire conversation (already formatted as
/// per-message blocks with sender + body).
pub fn summarize_thread(thread: &str) -> (String, String) {
    let system = "You are Komorebi's private email assistant. Summarize the conversation below in 3-5 concise sentences. Preserve key facts, dates, decisions, people involved, open questions, and any requested action. If you are uncertain about a fact, say so explicitly instead of inventing details. Reply with plain text only and no preamble.".to_string();
    let user = thread.trim().to_string();
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
        "You are Komorebi's private email assistant. Draft a reply to the email below. Tone: {}. Write only the reply body: no subject line, no greeting line that duplicates the recipient's name is required, and no signature block. If you are uncertain about a fact, say so explicitly instead of inventing details. Plain text only.",
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
        "You are Komorebi's private email assistant. Improve the draft below for clarity and correctness while preserving its meaning and the user's voice. Tone: {}. If you are uncertain about a fact, say so explicitly instead of inventing details. Return only the improved draft, with no preamble or explanation.",
        tone.trim()
    );
    let user = format!("Draft:\n\n{}", draft.trim());
    (system, user)
}

/// Build a triage prompt: classify into exactly one category.
pub fn triage(sender: Option<&str>, subject: Option<&str>, body: &str) -> (String, String) {
    let system = "You are Komorebi's private email assistant. Classify the email below into exactly one category: work, personal, finance, newsletters, or updates. If you are uncertain about a fact, say so explicitly instead of inventing details. Reply with only the single lowercase word and nothing else.".to_string();
    let user = format!(
        "From: {}\nSubject: {}\n\n{}",
        sender.unwrap_or("(unknown sender)"),
        subject.unwrap_or("(no subject)"),
        body.trim()
    );
    (system, user)
}

/// Build an action-item extraction prompt. The model is asked to return ONLY a
/// JSON array of action items (or `[]` when there are none), with optional
/// owner and due fields. Includes the uncertainty instruction.
pub fn extract_action_items(
    sender: Option<&str>,
    subject: Option<&str>,
    body: &str,
) -> (String, String) {
    let system = "You are Komorebi's private email assistant. Extract action items from the email below. Return ONLY a JSON array - no prose, no markdown fences. Each element must be an object of the shape {\"task\": string, \"owner\": string|null, \"due\": string|null}. If there are no action items, return []. If you are uncertain about a fact, say so explicitly instead of inventing details; in particular, omit owner or due when the email does not state them rather than guessing.".to_string();
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
    fn summarize_thread_uses_whole_conversation() {
        let (system, user) = summarize_thread("From: A\nbody one\n\nFrom: B\nbody two");
        assert!(system.contains("conversation"));
        assert!(user.contains("From: A"));
        assert!(user.contains("From: B"));
    }

    #[test]
    fn polish_returns_only_improved_draft() {
        let (system, user) = polish("Helo team,\nmeeting moved", "professional");
        assert!(system.contains("no preamble"));
        assert!(system.contains("professional"));
        assert!(user.contains("Helo team"));
    }

    #[test]
    fn all_prompts_include_uncertainty_instruction() {
        let needle =
            "If you are uncertain about a fact, say so explicitly instead of inventing details";
        let (s_summarize, _) = summarize(None, None, "body");
        let (s_thread, _) = summarize_thread("thread");
        let (s_draft, _) = draft_reply(None, None, "body", "brief");
        let (s_polish, _) = polish("draft", "professional");
        let (s_triage, _) = triage(None, None, "body");
        let (s_actions, _) = extract_action_items(None, None, "body");
        assert!(s_summarize.contains(needle));
        assert!(s_thread.contains(needle));
        assert!(s_draft.contains(needle));
        assert!(s_polish.contains(needle));
        assert!(s_triage.contains(needle));
        assert!(s_actions.contains(needle));
    }

    #[test]
    fn extract_action_items_prompt_asks_for_json_array() {
        let (system, user) = extract_action_items(
            Some("Pat"),
            Some("Kickoff"),
            "Please send the deck by Friday.",
        );
        assert!(system.contains("JSON array"));
        assert!(system.contains("[]"));
        assert!(user.contains("From: Pat"));
        assert!(user.contains("Subject: Kickoff"));
        assert!(user.contains("Please send the deck by Friday."));
    }
}

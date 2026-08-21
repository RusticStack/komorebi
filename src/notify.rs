//! Desktop notifications.
//!
//! Local-only, privacy-first notification surface for Komorebi. Notifications
//! fire on the user's machine only - there is **no push service**, no remote
//! endpoint, and nothing about a message leaves the device to display a toast.
//! The OS notification (via the `notify_rust` crate) is the only side effect,
//! and it is reached through a single function ([`show`]).
//!
//! Filtering is pure, deterministic, and fully testable without ever touching
//! the OS: [`NotificationConfig::should_notify`] is a pure decision over the
//! config + message attributes, with the current time injected only for the
//! quiet-hours comparison. The deterministic core is
//! [`NotificationConfig::should_notify_with_time`], which tests drive directly.
//!
//! Privacy boundary: this module never reads mailbox data, credentials, or the
//! filesystem. Callers pass the already-extracted sender/subject/priority in.

use anyhow::{Context, Result};
use chrono::Timelike;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Priority
// ---------------------------------------------------------------------------

/// Message priority used for notification filtering.
///
/// Serialized as lowercase strings (`"low"`, `"normal"`, `"high"`). The
/// derived `Ord` ordering is `Low < Normal < High`, so a rule with
/// `min_priority = Normal` matches `Normal` and `High` messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    /// Low priority - below the default notify threshold.
    Low,
    /// Normal priority - the default `min_priority` for rules.
    Normal,
    /// High priority - always satisfies a `Normal` or `High` threshold.
    High,
}

// ---------------------------------------------------------------------------
// NotificationRule
// ---------------------------------------------------------------------------

/// A single notification rule.
///
/// A rule *matches* a message when every specified selector agrees:
///
/// - `account_id` matches if `None` (wildcard) or equal to the message's
///   account.
/// - `folder_id` matches if `None` (wildcard) or equal to the message's
///   folder.
/// - `sender_email` matches if `None` (wildcard) or a case-insensitive exact
///   match against the message's sender email.
///
/// A matching rule grants a notification only when it is `enabled` **and** the
/// message `priority >= min_priority`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationRule {
    /// Stable identifier for this rule (caller-assigned; not interpreted).
    pub id: String,
    /// Restrict to one account, or `None` for all accounts.
    pub account_id: Option<String>,
    /// Restrict to one folder, or `None` for all folders.
    pub folder_id: Option<String>,
    /// Restrict to one sender email (case-insensitive exact match), or `None`
    /// for all senders.
    pub sender_email: Option<String>,
    /// Only notify when the message priority is at least this value.
    pub min_priority: Priority,
    /// Whether this rule is active.
    pub enabled: bool,
}

impl Default for NotificationRule {
    /// A catch-all rule: any account, any folder, any sender, `Normal`
    /// threshold, enabled. Useful as the fallback allow-rule when building a
    /// default-deny ruleset.
    fn default() -> Self {
        Self {
            id: String::new(),
            account_id: None,
            folder_id: None,
            sender_email: None,
            min_priority: Priority::Normal,
            enabled: true,
        }
    }
}

impl NotificationRule {
    /// Whether this rule's selectors match the given message attributes.
    ///
    /// This is a pure selector check: it does **not** consider `enabled` or
    /// `min_priority`. Those are applied by
    /// [`NotificationConfig::should_notify_with_time`].
    pub fn matches(&self, account_id: &str, folder_id: &str, sender_email: Option<&str>) -> bool {
        let account_ok = self.account_id.as_deref().is_none_or(|a| a == account_id);
        let folder_ok = self.folder_id.as_deref().is_none_or(|f| f == folder_id);
        let sender_ok = match self.sender_email.as_deref() {
            None => true,
            Some(wanted) => sender_email.is_some_and(|got| got.eq_ignore_ascii_case(wanted)),
        };
        account_ok && folder_ok && sender_ok
    }
}

// ---------------------------------------------------------------------------
// QuietHours
// ---------------------------------------------------------------------------

/// Do-not-disturb window in local wall-clock time.
///
/// `start_hour`/`end_hour` are local hours in `0..=23`. The quiet window is the
/// half-open interval `[start, end)`:
///
/// - `start <= end` is a same-day window, e.g. `9..17` quiets 09:00-16:59.
/// - `start > end` wraps overnight, e.g. `22..7` quiets 22:00-06:59.
/// - `start == end` is an empty (zero-length) window and is **never** quiet.
///   This is the safe default: an accidental `0..0` suppresses nothing rather
///   than everything.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuietHours {
    /// Whether quiet hours are active at all.
    pub enabled: bool,
    /// Local hour (0-23) at which the quiet window starts (inclusive).
    pub start_hour: u32,
    /// Local hour (0-23) at which the quiet window ends (exclusive).
    pub end_hour: u32,
}

impl Default for QuietHours {
    /// Disabled, defaulting to a 22:00-07:00 overnight window.
    fn default() -> Self {
        Self {
            enabled: false,
            start_hour: 22,
            end_hour: 7,
        }
    }
}

impl QuietHours {
    /// Pure, deterministic check against a local hour (0-23).
    ///
    /// Returns `true` when quiet hours are enabled and `local_hour` falls in
    /// `[start, end)`, handling the overnight wrap when `start > end`. This is
    /// the testable core; [`QuietHours::is_quiet`] delegates here after
    /// converting a UTC instant to the local hour.
    pub fn is_quiet_at(&self, local_hour: u32) -> bool {
        if !self.enabled {
            return false;
        }
        // Empty window: never quiet. Handled first so that start==end is not
        // misread by the overnight branch (which would otherwise be always-true).
        if self.start_hour == self.end_hour {
            return false;
        }
        if self.start_hour < self.end_hour {
            // Same-day window [start, end).
            local_hour >= self.start_hour && local_hour < self.end_hour
        } else {
            // Overnight wrap, e.g. 22..7 -> [start, 24) ∪ [0, end).
            local_hour >= self.start_hour || local_hour < self.end_hour
        }
    }

    /// Whether `now` (a UTC instant) falls inside the quiet window.
    ///
    /// The instant is converted to local wall-clock time (notifications are a
    /// local-time concept) and then checked with [`QuietHours::is_quiet_at`].
    /// Tests that need determinism should drive [`QuietHours::is_quiet_at`]
    /// directly, since this method depends on the host's local timezone.
    pub fn is_quiet(&self, now: chrono::DateTime<chrono::Utc>) -> bool {
        let local_hour = now.with_timezone(&chrono::Local).hour();
        self.is_quiet_at(local_hour)
    }
}

// ---------------------------------------------------------------------------
// NotificationConfig
// ---------------------------------------------------------------------------

/// Top-level notification configuration.
///
/// Serialization shape (serde JSON):
///
/// ```json
/// {
///   "enabled": true,
///   "rules": [ { "id": "...", "account_id": null, ... } ],
///   "quiet_hours": { "enabled": false, "start_hour": 22, "end_hour": 7 }
/// }
/// ```
///
/// ## `should_notify` decision logic
///
/// Given a message's `(account_id, folder_id, sender_email, priority)`:
///
/// 1. **Global gate.** If `self.enabled` is false → `false` (notifications off
///    entirely).
/// 2. **Quiet hours.** If [`QuietHours::is_quiet`] at the current local time →
///    `false` (suppressed, not an error).
/// 3. **Empty ruleset (default-allow).** If `rules` is empty → notify when
///    `priority >= Normal`, otherwise `false`.
/// 4. **Configured ruleset (default-deny).** If any rule matches the message
///    selectors, is `enabled`, and has `priority >= rule.min_priority` → `true`.
///    Otherwise → `false`.
///
/// Because step 4 is default-deny, **once you add any rule, only messages that
/// match an enabled rule with sufficient priority notify.** Add a catch-all
/// rule ([`NotificationRule::default`]: all-None selectors, `Normal` threshold)
/// as the last rule to let everything Normal+ through.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NotificationConfig {
    /// Global on/off switch. Derived `Default` is `false` (off): a privacy-first
    /// app does not notify until the user opts in.
    pub enabled: bool,
    /// Ordered rule list. See the struct docs for the matching semantics.
    pub rules: Vec<NotificationRule>,
    /// Do-not-disturb window.
    pub quiet_hours: QuietHours,
}

impl NotificationConfig {
    /// Notify decision for a message at the current local time.
    ///
    /// Thin wrapper over [`NotificationConfig::should_notify_with_time`] that
    /// injects `chrono::Utc::now()`. Non-deterministic across the wall clock
    /// and host timezone; prefer `should_notify_with_time` in tests.
    pub fn should_notify(
        &self,
        account_id: &str,
        folder_id: &str,
        sender_email: Option<&str>,
        priority: Priority,
    ) -> bool {
        self.should_notify_with_time(
            chrono::Utc::now(),
            account_id,
            folder_id,
            sender_email,
            priority,
        )
    }

    /// Deterministic notify decision for a message at an explicit instant.
    ///
    /// This is the testable core. See the [`NotificationConfig`] docs for the
    /// full decision logic.
    pub fn should_notify_with_time(
        &self,
        now: chrono::DateTime<chrono::Utc>,
        account_id: &str,
        folder_id: &str,
        sender_email: Option<&str>,
        priority: Priority,
    ) -> bool {
        // 1. Global gate.
        if !self.enabled {
            return false;
        }
        // 2. Quiet hours.
        if self.quiet_hours.is_quiet(now) {
            return false;
        }
        // 3. Empty ruleset: default-allow for Normal+.
        if self.rules.is_empty() {
            return priority >= Priority::Normal;
        }
        // 4. Configured ruleset: default-deny unless a matching enabled rule
        //    with sufficient priority grants the notification.
        self.rules.iter().any(|rule| {
            rule.enabled
                && priority >= rule.min_priority
                && rule.matches(account_id, folder_id, sender_email)
        })
    }
}

// ---------------------------------------------------------------------------
// OS surface
// ---------------------------------------------------------------------------

/// Fire a desktop notification via `notify_rust`.
///
/// This is the **only** function in the module that touches the OS. It is the
/// single side effect of the notification system; everything else is pure
/// filtering. Errors (no notification daemon, DBus/WinRT failure, etc.) are
/// wrapped with [`anyhow`].
pub fn show(title: &str, body: &str) -> Result<()> {
    notify_rust::Notification::new()
        .summary(title)
        .body(body)
        .show()
        .context("showing desktop notification")?;
    Ok(())
}

/// Main entry point: decide + display a notification for an incoming message.
///
/// Returns `Ok(true)` when a notification was shown, `Ok(false)` when it was
/// suppressed by the config (global off, quiet hours, or rule filtering).
/// Suppression is **not** an error. Only OS-level display failures propagate.
///
/// `sender` is always known for a real message, so it is passed as `Some`.
pub fn notify_message(
    config: &NotificationConfig,
    account_id: &str,
    folder_id: &str,
    sender: &str,
    subject: &str,
    priority: Priority,
) -> Result<bool> {
    if !config.should_notify(account_id, folder_id, Some(sender), priority) {
        return Ok(false);
    }
    show(sender, subject)?;
    Ok(true)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- Priority ordering ---------------------------------------------------

    #[test]
    fn priority_ordering() {
        assert!(Priority::Low < Priority::Normal);
        assert!(Priority::Normal < Priority::High);
        assert!(Priority::Low < Priority::High);
        assert_eq!(Priority::Normal, Priority::Normal);
    }

    #[test]
    fn priority_serde_lowercase() {
        assert_eq!(serde_json::to_string(&Priority::Low).unwrap(), "\"low\"");
        assert_eq!(
            serde_json::to_string(&Priority::Normal).unwrap(),
            "\"normal\""
        );
        assert_eq!(serde_json::to_string(&Priority::High).unwrap(), "\"high\"");
        for (text, expected) in [
            ("\"low\"", Priority::Low),
            ("\"normal\"", Priority::Normal),
            ("\"high\"", Priority::High),
        ] {
            assert_eq!(serde_json::from_str::<Priority>(text).unwrap(), expected);
        }
    }

    // -- QuietHours -----------------------------------------------------------

    #[test]
    fn quiet_hours_overnight_wrap() {
        // 22:00 -> 07:00 (start > end): hours 22..=6 quiet, 7 is exclusive end.
        let q = QuietHours {
            enabled: true,
            start_hour: 22,
            end_hour: 7,
        };
        assert!(q.is_quiet_at(22)); // start boundary, inclusive
        assert!(q.is_quiet_at(23));
        assert!(q.is_quiet_at(0));
        assert!(q.is_quiet_at(6));
        assert!(!q.is_quiet_at(7)); // exclusive end
        assert!(!q.is_quiet_at(21));
        assert!(!q.is_quiet_at(12));
    }

    #[test]
    fn quiet_hours_same_day_window() {
        // 09:00 -> 17:00 (start < end): 9..=16 quiet, 17 exclusive.
        let q = QuietHours {
            enabled: true,
            start_hour: 9,
            end_hour: 17,
        };
        assert!(q.is_quiet_at(9)); // inclusive start
        assert!(q.is_quiet_at(16));
        assert!(!q.is_quiet_at(17)); // exclusive end
        assert!(!q.is_quiet_at(8));
        assert!(!q.is_quiet_at(0));
        assert!(!q.is_quiet_at(23));
    }

    #[test]
    fn quiet_hours_disabled_never_quiet() {
        let q = QuietHours {
            enabled: false,
            start_hour: 0,
            end_hour: 23,
        };
        for h in 0..24 {
            assert!(
                !q.is_quiet_at(h),
                "hour {h} should not be quiet when disabled"
            );
        }
    }

    #[test]
    fn quiet_hours_empty_window_never_quiet() {
        // start == end is a zero-length window: never quiet (safe default).
        let q = QuietHours {
            enabled: true,
            start_hour: 22,
            end_hour: 22,
        };
        for h in 0..24 {
            assert!(
                !q.is_quiet_at(h),
                "hour {h} should not be quiet for empty window"
            );
        }
    }

    #[test]
    fn quiet_hours_default_values() {
        let q = QuietHours::default();
        assert!(!q.enabled);
        assert_eq!(q.start_hour, 22);
        assert_eq!(q.end_hour, 7);
        // Disabled -> never quiet regardless of hour.
        assert!(!q.is_quiet_at(23));
    }

    #[test]
    fn quiet_hours_is_quiet_disabled_is_timezone_independent() {
        // The disabled fast-path returns false before any local conversion, so
        // it is safe to assert against an arbitrary UTC instant.
        let q = QuietHours {
            enabled: false,
            start_hour: 22,
            end_hour: 7,
        };
        let now = chrono::Utc::now();
        assert!(!q.is_quiet(now));
    }

    #[test]
    fn quiet_hours_is_quiet_active_when_local_now_in_window() {
        // Deterministic regardless of host timezone: build the window around
        // the local hour of the *same* instant we pass to is_quiet.
        let now = chrono::Utc::now();
        let local_hour = now.with_timezone(&chrono::Local).hour();
        let end_hour = (local_hour + 1) % 24;
        let q = QuietHours {
            enabled: true,
            start_hour: local_hour,
            end_hour,
        };
        assert!(q.is_quiet(now));

        // A window that excludes the current local hour is not quiet.
        // Use the hour *after* end as the (exclusive) start of the non-quiet
        // gap: window [end, local_hour) leaves `local_hour` outside only when
        // start != end; construct a window covering every hour except
        // local_hour by setting start = (local_hour + 1) % 24, end = local_hour
        // (half-open, so local_hour is the exclusive end -> not quiet).
        let q_off = QuietHours {
            enabled: true,
            start_hour: (local_hour + 1) % 24,
            end_hour: local_hour,
        };
        assert!(!q_off.is_quiet(now));
    }

    // -- NotificationConfig::should_notify -----------------------------------

    fn enabled_cfg(rules: Vec<NotificationRule>) -> NotificationConfig {
        NotificationConfig {
            enabled: true,
            rules,
            quiet_hours: QuietHours::default(), // disabled
        }
    }

    #[test]
    fn should_notify_global_disabled() {
        let cfg = NotificationConfig {
            enabled: false,
            rules: vec![],
            quiet_hours: QuietHours::default(),
        };
        assert!(!cfg.should_notify_with_time(
            chrono::Utc::now(),
            "acc",
            "INBOX",
            Some("a@b.com"),
            Priority::High
        ));
    }

    #[test]
    fn should_notify_quiet_hours_active() {
        // Build a quiet window that contains the current local hour using the
        // same instant, so the assertion is deterministic on any host.
        let now = chrono::Utc::now();
        let local_hour = now.with_timezone(&chrono::Local).hour();
        let cfg = NotificationConfig {
            enabled: true,
            rules: vec![],
            quiet_hours: QuietHours {
                enabled: true,
                start_hour: local_hour,
                end_hour: (local_hour + 1) % 24,
            },
        };
        assert!(!cfg.should_notify_with_time(now, "acc", "INBOX", Some("a@b.com"), Priority::High));
    }

    #[test]
    fn should_notify_matching_rule_sufficient_priority() {
        let cfg = enabled_cfg(vec![NotificationRule {
            id: "r1".into(),
            account_id: Some("acc".into()),
            folder_id: None,
            sender_email: None,
            min_priority: Priority::Normal,
            enabled: true,
        }]);
        assert!(cfg.should_notify_with_time(
            chrono::Utc::now(),
            "acc",
            "INBOX",
            Some("a@b.com"),
            Priority::High
        ));
        assert!(cfg.should_notify_with_time(
            chrono::Utc::now(),
            "acc",
            "INBOX",
            Some("a@b.com"),
            Priority::Normal
        ));
    }

    #[test]
    fn should_notify_rule_higher_min_priority() {
        // Rule requires High; a Normal message does not satisfy it, and since
        // rules are configured (default-deny) this suppresses the notification.
        let cfg = enabled_cfg(vec![NotificationRule {
            id: "r1".into(),
            account_id: None,
            folder_id: None,
            sender_email: None,
            min_priority: Priority::High,
            enabled: true,
        }]);
        assert!(!cfg.should_notify_with_time(
            chrono::Utc::now(),
            "acc",
            "INBOX",
            Some("a@b.com"),
            Priority::Normal
        ));
        // A High message does satisfy it.
        assert!(cfg.should_notify_with_time(
            chrono::Utc::now(),
            "acc",
            "INBOX",
            Some("a@b.com"),
            Priority::High
        ));
    }

    #[test]
    fn should_notify_sender_specific_rule_matches() {
        let cfg = enabled_cfg(vec![NotificationRule {
            id: "vip".into(),
            account_id: None,
            folder_id: None,
            sender_email: Some("boss@company.com".into()),
            min_priority: Priority::Normal,
            enabled: true,
        }]);
        // Matching sender (case-insensitive) -> notify.
        assert!(cfg.should_notify_with_time(
            chrono::Utc::now(),
            "acc",
            "INBOX",
            Some("BOSS@company.com"),
            Priority::Normal
        ));
        // Non-matching sender -> default-deny (rules configured, no match).
        assert!(!cfg.should_notify_with_time(
            chrono::Utc::now(),
            "acc",
            "INBOX",
            Some("random@example.com"),
            Priority::Normal
        ));
    }

    #[test]
    fn should_notify_sender_rule_requires_some_sender() {
        // A rule that pins a sender must not match when the message has no
        // sender (None).
        let cfg = enabled_cfg(vec![NotificationRule {
            id: "vip".into(),
            account_id: None,
            folder_id: None,
            sender_email: Some("boss@company.com".into()),
            min_priority: Priority::Normal,
            enabled: true,
        }]);
        assert!(!cfg.should_notify_with_time(
            chrono::Utc::now(),
            "acc",
            "INBOX",
            None,
            Priority::High
        ));
    }

    #[test]
    fn should_notify_no_rules_normal_priority() {
        let cfg = enabled_cfg(vec![]);
        assert!(cfg.should_notify_with_time(
            chrono::Utc::now(),
            "acc",
            "INBOX",
            Some("a@b.com"),
            Priority::Normal
        ));
        assert!(cfg.should_notify_with_time(
            chrono::Utc::now(),
            "acc",
            "INBOX",
            Some("a@b.com"),
            Priority::High
        ));
    }

    #[test]
    fn should_notify_no_rules_low_priority() {
        let cfg = enabled_cfg(vec![]);
        assert!(!cfg.should_notify_with_time(
            chrono::Utc::now(),
            "acc",
            "INBOX",
            Some("a@b.com"),
            Priority::Low
        ));
    }

    #[test]
    fn should_notify_disabled_rule_does_not_grant() {
        // A matching but disabled rule does not notify (default-deny).
        let cfg = enabled_cfg(vec![NotificationRule {
            id: "off".into(),
            account_id: None,
            folder_id: None,
            sender_email: None,
            min_priority: Priority::Normal,
            enabled: false,
        }]);
        assert!(!cfg.should_notify_with_time(
            chrono::Utc::now(),
            "acc",
            "INBOX",
            Some("a@b.com"),
            Priority::High
        ));
    }

    #[test]
    fn should_notify_catchall_fallback_allows_normal() {
        // A specific deny-ish rule (High threshold) plus a catch-all default
        // rule should let a Normal message through via the catch-all.
        let cfg = enabled_cfg(vec![
            NotificationRule {
                id: "urgent".into(),
                account_id: Some("work".into()),
                folder_id: None,
                sender_email: None,
                min_priority: Priority::High,
                enabled: true,
            },
            NotificationRule::default(),
        ]);
        // Normal message to a non-"work" account: matched by the catch-all.
        assert!(cfg.should_notify_with_time(
            chrono::Utc::now(),
            "personal",
            "INBOX",
            Some("a@b.com"),
            Priority::Normal
        ));
    }

    #[test]
    fn should_notify_account_selector() {
        let cfg = enabled_cfg(vec![NotificationRule {
            id: "work-only".into(),
            account_id: Some("work".into()),
            folder_id: None,
            sender_email: None,
            min_priority: Priority::Normal,
            enabled: true,
        }]);
        assert!(cfg.should_notify_with_time(
            chrono::Utc::now(),
            "work",
            "INBOX",
            Some("a@b.com"),
            Priority::Normal
        ));
        // Other account: no matching rule -> default-deny.
        assert!(!cfg.should_notify_with_time(
            chrono::Utc::now(),
            "personal",
            "INBOX",
            Some("a@b.com"),
            Priority::Normal
        ));
    }

    // -- serde roundtrip -----------------------------------------------------

    #[test]
    fn config_serde_roundtrip() {
        let cfg = NotificationConfig {
            enabled: true,
            rules: vec![
                NotificationRule {
                    id: "r1".into(),
                    account_id: Some("acc".into()),
                    folder_id: None,
                    sender_email: Some("boss@company.com".into()),
                    min_priority: Priority::High,
                    enabled: true,
                },
                NotificationRule::default(),
            ],
            quiet_hours: QuietHours {
                enabled: true,
                start_hour: 22,
                end_hour: 7,
            },
        };
        let json = serde_json::to_string(&cfg).expect("serialize");
        let back: NotificationConfig = serde_json::from_str(&json).expect("deserialize");
        // Re-serialize for a stable equality check (structs are not PartialEq).
        let json2 = serde_json::to_string(&back).expect("reserialize");
        assert_eq!(json, json2);

        // Spot-check key fields.
        assert!(back.enabled);
        assert_eq!(back.rules.len(), 2);
        assert!(back.quiet_hours.enabled);
        assert_eq!(back.quiet_hours.start_hour, 22);
        assert_eq!(back.quiet_hours.end_hour, 7);
        assert_eq!(back.rules[0].min_priority, Priority::High);
        assert_eq!(
            back.rules[0].sender_email.as_deref(),
            Some("boss@company.com")
        );
    }

    #[test]
    fn config_default_is_off_with_empty_rules() {
        let cfg = NotificationConfig::default();
        assert!(!cfg.enabled);
        assert!(cfg.rules.is_empty());
        assert!(!cfg.quiet_hours.enabled);
        assert_eq!(cfg.quiet_hours.start_hour, 22);
        assert_eq!(cfg.quiet_hours.end_hour, 7);
    }

    #[test]
    fn rule_default_is_catchall_normal() {
        let r = NotificationRule::default();
        assert!(r.enabled);
        assert_eq!(r.min_priority, Priority::Normal);
        assert!(r.account_id.is_none());
        assert!(r.folder_id.is_none());
        assert!(r.sender_email.is_none());
        // Matches everything.
        assert!(r.matches("any", "any", Some("any@example.com")));
        assert!(r.matches("any", "any", None));
    }
}

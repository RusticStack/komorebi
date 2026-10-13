//! Every sentence of `ui/tr.slint`, called once with sample arguments by the
//! render test in `text.rs`. Keep in step with the file: the test fails for
//! a function missing here (and for one that is not in the file any more).
//! Strings are the tokens `t0`, `t1`, ... in the order of the parameters, so
//! the test can see that each came through; numbers are 3.

use crate::Tr;
use slint::SharedString;

/// A call of one `Tr` function.
pub(super) type Call = fn(&Tr<'_>) -> SharedString;

pub(super) const CALLS: &[(&str, Call)] = &[
    ("window-title", |t| t.invoke_window_title(3)),
    ("start-failed", |t| t.invoke_start_failed("t0".into())),
    ("crash-notice", |t| t.invoke_crash_notice()),
    ("cache-damaged", |t| t.invoke_cache_damaged("t0".into())),
    ("wipe-files-left", |t| t.invoke_wipe_files_left("t0".into())),
    ("say-offline", |t| t.invoke_say_offline()),
    ("say-auth", |t| t.invoke_say_auth()),
    ("say-tls", |t| t.invoke_say_tls()),
    ("say-too-large", |t| t.invoke_say_too_large()),
    ("say-quota", |t| t.invoke_say_quota()),
    ("say-busy", |t| t.invoke_say_busy()),
    ("say-not-found", |t| t.invoke_say_not_found("t0".into())),
    ("say-no-answer", |t| {
        t.invoke_say_no_answer("t0".into(), "t1".into())
    }),
    ("say-lost", |t| t.invoke_say_lost("t0".into())),
    ("say-rejected", |t| {
        t.invoke_say_rejected("t0".into(), "t1".into())
    }),
    ("say-rejected-preset", |t| {
        t.invoke_say_rejected_preset("t0".into(), "t1".into())
    }),
    ("say-other-preset", |t| {
        t.invoke_say_other_preset("t0".into())
    }),
    ("say-secure-failed", |t| {
        t.invoke_say_secure_failed("t0".into())
    }),
    ("say-no-connection", |t| {
        t.invoke_say_no_connection("t0".into(), "t1".into())
    }),
    ("say-no-secure", |t| t.invoke_say_no_secure("t0".into())),
    ("say-no-sign-in", |t| {
        t.invoke_say_no_sign_in("t0".into(), "t1".into())
    }),
    ("say-no-folders", |t| t.invoke_say_no_folders("t0".into())),
    ("say-no-starttls", |t| t.invoke_say_no_starttls("t0".into())),
    ("say-plain-answer", |t| t.invoke_say_plain_answer()),
    ("say-no-auth", |t| t.invoke_say_no_auth("t0".into())),
    ("say-bad-greeting", |t| {
        t.invoke_say_bad_greeting("t0".into())
    }),
    ("say-no-login", |t| t.invoke_say_no_login("t0".into())),
    ("say-utf8-only", |t| t.invoke_say_utf8_only("t0".into())),
    ("say-cert-changed", |t| {
        t.invoke_say_cert_changed("t0".into())
    }),
    ("say-cert-changed-short", |t| {
        t.invoke_say_cert_changed_short("t0".into())
    }),
    ("say-sender-refused", |t| {
        t.invoke_say_sender_refused("t0".into(), "t1".into())
    }),
    ("say-sign-in-latched", |t| t.invoke_say_sign_in_latched()),
    ("say-unconfirmed", |t| t.invoke_say_unconfirmed("t0".into())),
    ("say-too-large-send", |t| {
        t.invoke_say_too_large_send("t0".into())
    }),
    ("notice-left-flagged", |t| t.invoke_notice_left_flagged()),
    ("notice-move-gone", |t| t.invoke_notice_move_gone()),
    ("notice-no-drafts-folder", |t| {
        t.invoke_notice_no_drafts_folder()
    }),
    ("notice-no-sent-folder", |t| {
        t.invoke_notice_no_sent_folder()
    }),
    ("notice-no-draft-uid", |t| t.invoke_notice_no_draft_uid()),
    ("notice-drafts-not-saved", |t| {
        t.invoke_notice_drafts_not_saved("t0".into())
    }),
    ("account-line", |t| {
        t.invoke_account_line("t0".into(), "t1".into())
    }),
    ("error-detail", |t| t.invoke_error_detail("t0".into())),
    ("not-saved", |t| t.invoke_not_saved("t0".into())),
    ("not-removed", |t| t.invoke_not_removed("t0".into())),
    ("not-reset", |t| t.invoke_not_reset("t0".into())),
    ("stopped-no-files", |t| {
        t.invoke_stopped_no_files("t0".into())
    }),
    ("could-not-save", |t| t.invoke_could_not_save("t0".into())),
    ("could-not-save-file", |t| {
        t.invoke_could_not_save_file("t0".into(), "t1".into())
    }),
    ("could-not-open-file", |t| {
        t.invoke_could_not_open_file("t0".into(), "t1".into())
    }),
    ("could-not-attach", |t| {
        t.invoke_could_not_attach("t0".into())
    }),
    ("could-not-open-draft", |t| {
        t.invoke_could_not_open_draft("t0".into())
    }),
    ("could-not-save-draft", |t| {
        t.invoke_could_not_save_draft("t0".into())
    }),
    ("could-not-send", |t| t.invoke_could_not_send("t0".into())),
    ("could-not-send-back-in-drafts", |t| {
        t.invoke_could_not_send_back_in_drafts("t0".into())
    }),
    ("could-not-undo", |t| t.invoke_could_not_undo("t0".into())),
    ("could-not-send-reply", |t| {
        t.invoke_could_not_send_reply("t0".into())
    }),
    ("could-not-download-status", |t| {
        t.invoke_could_not_download_status("t0".into())
    }),
    ("could-not-download-body", |t| {
        t.invoke_could_not_download_body("t0".into())
    }),
    ("saved-to", |t| t.invoke_saved_to("t0".into())),
    ("no-app-opens", |t| t.invoke_no_app_opens("t0".into())),
    ("file-can-run", |t| t.invoke_file_can_run("t0".into())),
    ("sent", |t| t.invoke_sent()),
    ("sent-not-saved", |t| t.invoke_sent_not_saved("t0".into())),
    ("not-sent-yet", |t| t.invoke_not_sent_yet("t0".into())),
    ("not-sent", |t| t.invoke_not_sent("t0".into())),
    ("message-discarded", |t| t.invoke_message_discarded()),
    ("rebuild-failed", |t| t.invoke_rebuild_failed()),
    ("sending-in", |t| t.invoke_sending_in(3)),
    ("sending-subject-in", |t| {
        t.invoke_sending_subject_in("t0".into(), 3)
    }),
    ("sending", |t| t.invoke_sending()),
    ("outbox-pending", |t| t.invoke_outbox_pending()),
    ("outbox-waiting", |t| t.invoke_outbox_waiting()),
    ("outbox-retry", |t| {
        t.invoke_outbox_retry("t0".into(), "t1".into())
    }),
    ("send-no-such", |t| t.invoke_send_no_such("t0".into())),
    ("send-no-such-any", |t| t.invoke_send_no_such_any()),
    ("send-bad-address", |t| t.invoke_send_bad_address()),
    ("send-mailbox-full", |t| t.invoke_send_mailbox_full()),
    ("send-too-large", |t| t.invoke_send_too_large()),
    ("send-sender-refused", |t| t.invoke_send_sender_refused()),
    ("send-policy", |t| t.invoke_send_policy()),
    ("send-sign-in", |t| t.invoke_send_sign_in()),
    ("send-certificate", |t| t.invoke_send_certificate()),
    ("send-busy", |t| t.invoke_send_busy()),
    ("send-busy-gave-up", |t| t.invoke_send_busy_gave_up()),
    ("send-unreachable", |t| t.invoke_send_unreachable()),
    ("send-unreachable-gave-up", |t| {
        t.invoke_send_unreachable_gave_up()
    }),
    ("send-other", |t| t.invoke_send_other()),
    ("send-unconfirmed", |t| t.invoke_send_unconfirmed()),
    ("undo-too-late", |t| t.invoke_undo_too_late()),
    ("reply-in-outbox", |t| t.invoke_reply_in_outbox()),
    ("answer-kept", |t| t.invoke_answer_kept()),
    ("action-undo", |t| t.invoke_action_undo()),
    ("action-retry", |t| t.invoke_action_retry()),
    ("action-review-certificate", |t| {
        t.invoke_action_review_certificate()
    }),
    ("action-try-now", |t| t.invoke_action_try_now()),
    ("nav-unreachable", |t| t.invoke_nav_unreachable("t0".into())),
    ("action-update-password", |t| {
        t.invoke_action_update_password()
    }),
    ("whats-new-bar", |t| t.invoke_whats_new_bar("t0".into())),
    ("action-whats-new", |t| t.invoke_action_whats_new()),
    ("err-email", |t| t.invoke_err_email()),
    ("err-email-first", |t| t.invoke_err_email_first()),
    ("err-account-exists", |t| t.invoke_err_account_exists()),
    ("err-account-removed", |t| t.invoke_err_account_removed()),
    ("err-password-to-test", |t| t.invoke_err_password_to_test()),
    ("err-offline-check", |t| t.invoke_err_offline_check()),
    ("err-offline-cert", |t| t.invoke_err_offline_cert()),
    ("err-offline-find", |t| t.invoke_err_offline_find()),
    ("err-saved-password", |t| t.invoke_err_saved_password()),
    ("err-provider", |t| t.invoke_err_provider()),
    ("err-server-names", |t| t.invoke_err_server_names()),
    ("err-server-name-only", |t| t.invoke_err_server_name_only()),
    ("err-zone-id", |t| t.invoke_err_zone_id()),
    ("err-ascii-name", |t| t.invoke_err_ascii_name()),
    ("err-port", |t| t.invoke_err_port("t0".into())),
    ("err-username", |t| t.invoke_err_username("t0".into())),
    ("check-passed", |t| t.invoke_check_passed()),
    ("account-added", |t| t.invoke_account_added("t0".into())),
    ("account-saved", |t| t.invoke_account_saved("t0".into())),
    ("account-off", |t| t.invoke_account_off()),
    ("account-downloading", |t| {
        t.invoke_account_downloading("t0".into(), "t1".into())
    }),
    ("account-sign-in-rejected", |t| {
        t.invoke_account_sign_in_rejected()
    }),
    ("account-secure-failed", |t| {
        t.invoke_account_secure_failed()
    }),
    ("account-syncing", |t| t.invoke_account_syncing()),
    ("account-checked", |t| t.invoke_account_checked("t0".into())),
    ("account-up-to-date", |t| t.invoke_account_up_to_date()),
    ("account-unreachable", |t| {
        t.invoke_account_unreachable("t0".into())
    }),
    ("account-sync-failed", |t| {
        t.invoke_account_sync_failed("t0".into())
    }),
    ("status-downloading", |t| {
        t.invoke_status_downloading("t0".into(), "t1".into(), "t2".into(), "t3".into())
    }),
    ("redownloading", |t| t.invoke_redownloading("t0".into())),
    ("redownload-later", |t| {
        t.invoke_redownload_later("t0".into())
    }),
    ("all-inboxes", |t| t.invoke_all_inboxes()),
    ("drafts", |t| t.invoke_drafts()),
    ("outbox", |t| t.invoke_outbox()),
    ("all-folders", |t| t.invoke_all_folders()),
    ("search-in", |t| t.invoke_search_in("t0".into())),
    ("scope-this-folder", |t| t.invoke_scope_this_folder()),
    ("scope-these-inboxes", |t| t.invoke_scope_these_inboxes()),
    ("search-results", |t| t.invoke_search_results()),
    ("results-in", |t| t.invoke_results_in(3, "t0".into())),
    ("results-all", |t| t.invoke_results_all(3)),
    ("show-in-folder", |t| t.invoke_show_in_folder()),
    ("account-folder", |t| {
        t.invoke_account_folder("t0".into(), "t1".into())
    }),
    ("menu-mark-all-read", |t| t.invoke_menu_mark_all_read()),
    ("menu-move-up", |t| t.invoke_menu_move_up()),
    ("menu-move-down", |t| t.invoke_menu_move_down()),
    ("menu-select-messages", |t| t.invoke_menu_select_messages()),
    ("menu-reply", |t| t.invoke_menu_reply()),
    ("menu-reply-all", |t| t.invoke_menu_reply_all()),
    ("menu-forward", |t| t.invoke_menu_forward()),
    ("menu-forward-attached", |t| {
        t.invoke_menu_forward_attached()
    }),
    ("menu-mark-read", |t| t.invoke_menu_mark_read()),
    ("menu-mark-unread", |t| t.invoke_menu_mark_unread()),
    ("menu-flag", |t| t.invoke_menu_flag()),
    ("menu-remove-flag", |t| t.invoke_menu_remove_flag()),
    ("menu-archive", |t| t.invoke_menu_archive()),
    ("menu-delete", |t| t.invoke_menu_delete()),
    ("menu-move-to", |t| t.invoke_menu_move_to()),
    ("menu-report-junk", |t| t.invoke_menu_report_junk()),
    ("menu-not-junk", |t| t.invoke_menu_not_junk()),
    ("menu-save-eml", |t| t.invoke_menu_save_eml()),
    ("menu-show-original", |t| t.invoke_menu_show_original()),
    ("menu-copy-message-id", |t| t.invoke_menu_copy_message_id()),
    ("menu-plain-text", |t| t.invoke_menu_plain_text()),
    ("menu-original-colours", |t| {
        t.invoke_menu_original_colours()
    }),
    ("menu-load-images", |t| t.invoke_menu_load_images()),
    ("menu-stop-images", |t| t.invoke_menu_stop_images()),
    ("key-backspace", |t| t.invoke_key_backspace()),
    ("move-recent", |t| t.invoke_move_recent()),
    ("menu-collapse-all", |t| t.invoke_menu_collapse_all()),
    ("menu-collapse-all-folders", |t| {
        t.invoke_menu_collapse_all_folders()
    }),
    ("menu-refresh", |t| t.invoke_menu_refresh()),
    ("menu-account-settings", |t| {
        t.invoke_menu_account_settings()
    }),
    ("rail-new-mail", |t| t.invoke_rail_new_mail()),
    ("rail-settings", |t| t.invoke_rail_settings()),
    ("rail-folders", |t| t.invoke_rail_folders()),
    ("rail-all-inboxes", |t| t.invoke_rail_all_inboxes()),
    ("role-inbox", |t| t.invoke_role_inbox()),
    ("role-drafts", |t| t.invoke_role_drafts()),
    ("role-sent", |t| t.invoke_role_sent()),
    ("role-archive", |t| t.invoke_role_archive()),
    ("role-junk", |t| t.invoke_role_junk()),
    ("role-trash", |t| t.invoke_role_trash()),
    ("empty-no-matches", |t| t.invoke_empty_no_matches()),
    ("empty-no-drafts", |t| t.invoke_empty_no_drafts()),
    ("empty-outbox", |t| t.invoke_empty_outbox()),
    ("empty-no-unread", |t| t.invoke_empty_no_unread()),
    ("empty-no-flagged", |t| t.invoke_empty_no_flagged()),
    ("empty-loading", |t| t.invoke_empty_loading()),
    ("empty-unreachable", |t| t.invoke_empty_unreachable()),
    ("empty-failed", |t| t.invoke_empty_failed()),
    ("empty-nothing", |t| t.invoke_empty_nothing()),
    ("empty-folder", |t| t.invoke_empty_folder()),
    ("empty-downloading", |t| {
        t.invoke_empty_downloading("t0".into(), "t1".into())
    }),
    ("empty-downloading-start", |t| {
        t.invoke_empty_downloading_start()
    }),
    ("removed-deleted", |t| t.invoke_removed_deleted()),
    ("removed-moved", |t| t.invoke_removed_moved("t0".into())),
    ("removed-deleted-many", |t| t.invoke_removed_deleted_many(3)),
    ("removed-moved-many", |t| {
        t.invoke_removed_moved_many(3, "t0".into())
    }),
    ("removed-with-note", |t| {
        t.invoke_removed_with_note("t0".into(), "t1".into())
    }),
    ("missing-archive", |t| t.invoke_missing_archive()),
    ("missing-trash", |t| t.invoke_missing_trash()),
    ("missing-junk", |t| t.invoke_missing_junk()),
    ("missing-inbox", |t| t.invoke_missing_inbox()),
    ("role-automatic", |t| t.invoke_role_automatic("t0".into())),
    ("role-automatic-none", |t| t.invoke_role_automatic_none()),
    ("row-no-recipient", |t| t.invoke_row_no_recipient()),
    ("row-to", |t| t.invoke_row_to("t0".into())),
    ("no-subject", |t| t.invoke_no_subject()),
    ("body-cut-note", |t| t.invoke_body_cut_note()),
    ("encrypted-notice", |t| {
        t.invoke_encrypted_notice("t0".into())
    }),
    ("reader-too-large", |t| t.invoke_reader_too_large()),
    ("reader-too-slow", |t| t.invoke_reader_too_slow()),
    ("link-text-mismatch", |t| {
        t.invoke_link_text_mismatch("t0".into(), "t1".into())
    }),
    ("link-punycode", |t| t.invoke_link_punycode("t0".into())),
    ("link-user-info", |t| t.invoke_link_user_info("t0".into())),
    ("link-raw-ip", |t| t.invoke_link_raw_ip("t0".into())),
    ("trust-notice-one", |t| {
        t.invoke_trust_notice_one("t0".into())
    }),
    ("trust-notice-two", |t| {
        t.invoke_trust_notice_two("t0".into(), "t1".into())
    }),
    ("trust-dmarc", |t| t.invoke_trust_dmarc()),
    ("trust-spf-dkim", |t| t.invoke_trust_spf_dkim()),
    ("trust-name-domain", |t| {
        t.invoke_trust_name_domain("t0".into(), "t1".into())
    }),
    ("trust-punycode", |t| {
        t.invoke_trust_punycode("t0".into(), "t1".into())
    }),
    ("trust-mixed-script", |t| {
        t.invoke_trust_mixed_script("t0".into(), "t1".into())
    }),
    ("script-cyrillic", |t| t.invoke_script_cyrillic()),
    ("script-greek", |t| t.invoke_script_greek()),
    ("fetching", |t| t.invoke_fetching()),
    ("not-downloaded", |t| t.invoke_not_downloaded()),
    ("no-message-id", |t| t.invoke_no_message_id()),
    ("message-id-copied", |t| t.invoke_message_id_copied()),
    ("invite-cancelled", |t| t.invoke_invite_cancelled()),
    ("invite-accepted", |t| t.invoke_invite_accepted()),
    ("invite-tentative", |t| t.invoke_invite_tentative()),
    ("invite-declined", |t| t.invoke_invite_declined()),
    ("invite-over", |t| t.invoke_invite_over()),
    ("invite-updated", |t| t.invoke_invite_updated()),
    ("invite-organizer", |t| {
        t.invoke_invite_organizer("t0".into())
    }),
    ("invite-where", |t| t.invoke_invite_where("t0".into())),
    ("invite-no-title", |t| t.invoke_invite_no_title()),
    ("title-new", |t| t.invoke_title_new()),
    ("title-reply", |t| t.invoke_title_reply()),
    ("title-reply-all", |t| t.invoke_title_reply_all()),
    ("title-forward", |t| t.invoke_title_forward()),
    ("fix-address-to", |t| t.invoke_fix_address_to()),
    ("fix-address-cc", |t| t.invoke_fix_address_cc()),
    ("fix-address-bcc", |t| t.invoke_fix_address_bcc()),
    ("add-recipient", |t| t.invoke_add_recipient()),
    ("attachments-over", |t| t.invoke_attachments_over()),
    ("guard-both", |t| t.invoke_guard_both()),
    ("guard-no-subject", |t| t.invoke_guard_no_subject()),
    ("guard-no-attachment", |t| t.invoke_guard_no_attachment()),
    ("address-invalid", |t| t.invoke_address_invalid("t0".into())),
    ("earlier-draft-saved", |t| t.invoke_earlier_draft_saved()),
    ("add-account-write", |t| t.invoke_add_account_write()),
    ("add-account-send", |t| t.invoke_add_account_send()),
    ("still-downloading", |t| t.invoke_still_downloading()),
    ("draft-gone", |t| t.invoke_draft_gone()),
    ("draft-left-out", |t| t.invoke_draft_left_out("t0".into())),
    ("draft-saved", |t| t.invoke_draft_saved()),
    ("draft-saved-at", |t| t.invoke_draft_saved_at("t0".into())),
    ("quote-wrote", |t| {
        t.invoke_quote_wrote("t0".into(), "t1".into())
    }),
    ("forwarded", |t| t.invoke_forwarded()),
    ("forward-from", |t| t.invoke_forward_from()),
    ("forward-date", |t| t.invoke_forward_date()),
    ("forward-subject", |t| t.invoke_forward_subject()),
    ("forward-to", |t| t.invoke_forward_to()),
    ("attach-total", |t| t.invoke_attach_total("t0".into())),
    ("attach-folder", |t| t.invoke_attach_folder("t0".into())),
    ("attach-too-big", |t| t.invoke_attach_too_big("t0".into())),
    ("attach-failed", |t| {
        t.invoke_attach_failed("t0".into(), "t1".into())
    }),
    ("cert-pinned", |t| t.invoke_cert_pinned("t0".into())),
    ("cert-damaged", |t| t.invoke_cert_damaged("t0".into())),
    ("cert-windows", |t| t.invoke_cert_windows()),
    ("cert-unreadable", |t| t.invoke_cert_unreadable("t0".into())),
    ("cert-trusted", |t| t.invoke_cert_trusted("t0".into())),
    ("cert-names", |t| t.invoke_cert_names("t0".into())),
    ("cert-subject", |t| t.invoke_cert_subject("t0".into())),
    ("cert-self-signed", |t| t.invoke_cert_self_signed()),
    ("cert-issued-by", |t| t.invoke_cert_issued_by("t0".into())),
    ("cert-valid", |t| {
        t.invoke_cert_valid("t0".into(), "t1".into())
    }),
    ("cert-fp-label", |t| t.invoke_cert_fp_label()),
    ("cert-fp-new", |t| t.invoke_cert_fp_new()),
    ("cert-fp-old", |t| t.invoke_cert_fp_old()),
    ("cert-fp-caution", |t| t.invoke_cert_fp_caution()),
    ("find-none", |t| t.invoke_find_none()),
    ("find-builtin", |t| t.invoke_find_builtin()),
    ("find-srv", |t| t.invoke_find_srv()),
    ("find-autoconfig", |t| t.invoke_find_autoconfig()),
    ("find-both", |t| t.invoke_find_both()),
    ("find-no-incoming", |t| t.invoke_find_no_incoming()),
    ("find-no-sending", |t| t.invoke_find_no_sending()),
    ("find-refused", |t| t.invoke_find_refused("t0".into())),
    ("find-kept", |t| t.invoke_find_kept()),
    ("q-resend-title", |t| t.invoke_q_resend_title()),
    ("q-resend-text", |t| t.invoke_q_resend_text()),
    ("q-resend-ok", |t| t.invoke_q_resend_ok()),
    ("q-discard-title", |t| t.invoke_q_discard_title()),
    ("q-discard-text", |t| t.invoke_q_discard_text()),
    ("q-discard-ok", |t| t.invoke_q_discard_ok()),
    ("q-delete-trash-title", |t| t.invoke_q_delete_trash_title(3)),
    ("q-delete-trash-text", |t| t.invoke_q_delete_trash_text(3)),
    ("q-delete-junk-title", |t| t.invoke_q_delete_junk_title(3)),
    ("q-delete-junk-text", |t| t.invoke_q_delete_junk_text(3)),
    ("q-delete-ok", |t| t.invoke_q_delete_ok()),
    ("q-remove-title", |t| t.invoke_q_remove_title("t0".into())),
    ("q-remove-text", |t| t.invoke_q_remove_text()),
    ("q-remove-unsent", |t| t.invoke_q_remove_unsent(3)),
    ("q-remove-ok", |t| t.invoke_q_remove_ok()),
    ("q-rsvp-title-accepted", |t| {
        t.invoke_q_rsvp_title_accepted()
    }),
    ("q-rsvp-title-tentative", |t| {
        t.invoke_q_rsvp_title_tentative()
    }),
    ("q-rsvp-title-declined", |t| {
        t.invoke_q_rsvp_title_declined()
    }),
    ("q-rsvp-text", |t| {
        t.invoke_q_rsvp_text("t0".into(), "t1".into())
    }),
    ("q-rsvp-domain", |t| {
        t.invoke_q_rsvp_domain("t0".into(), "t1".into())
    }),
    ("q-rsvp-unlisted", |t| t.invoke_q_rsvp_unlisted("t0".into())),
    ("q-rsvp-alias", |t| t.invoke_q_rsvp_alias("t0".into())),
    ("q-rsvp-local", |t| t.invoke_q_rsvp_local()),
    ("q-rsvp-ok", |t| t.invoke_q_rsvp_ok()),
    ("q-rsvp-alt", |t| t.invoke_q_rsvp_alt()),
    ("q-redownload-title", |t| {
        t.invoke_q_redownload_title("t0".into())
    }),
    ("q-redownload-text", |t| t.invoke_q_redownload_text()),
    ("q-redownload-ok", |t| t.invoke_q_redownload_ok()),
    ("q-wipe-title", |t| t.invoke_q_wipe_title()),
    ("q-wipe-text", |t| t.invoke_q_wipe_text()),
    ("q-wipe-unsent", |t| t.invoke_q_wipe_unsent(3)),
    ("q-wipe-ok", |t| t.invoke_q_wipe_ok()),
    ("q-trust-title", |t| t.invoke_q_trust_title()),
    ("q-trust-text", |t| {
        t.invoke_q_trust_text("t0".into(), "t1".into())
    }),
    ("q-trust-ok", |t| t.invoke_q_trust_ok()),
    ("q-trust-note", |t| t.invoke_q_trust_note()),
    ("dlg-save-attachment", |t| t.invoke_dlg_save_attachment()),
    ("dlg-save-invitation", |t| t.invoke_dlg_save_invitation()),
    ("dlg-save-message", |t| t.invoke_dlg_save_message()),
    ("dlg-attach-files", |t| t.invoke_dlg_attach_files()),
];

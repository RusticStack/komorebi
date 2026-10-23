//! Settings > Rules on the real app, headless (the `Harness` of
//! `composer::tests`): saving writes the rule, the toggle and the order
//! persist, Delete asks first.

use super::*;
use crate::RulesUi;

#[test]
fn rules_are_saved_toggled_reordered_and_deleted_in_settings() {
    let h = Harness::new("rules-settings");
    let ui = h.ui();
    let g = ui.global::<RulesUi>();
    let account = h.app(|a| a.accounts[0].id);
    ui.set_settings_section(2);
    ui.set_show_settings(true);
    h.app(|a| a.reload_rules());
    h.frame();
    assert_eq!(g.get_items().row_count(), 0);

    // A new rule needs a value and an action.
    g.invoke_add();
    h.frame();
    assert!(g.get_editing());
    g.set_name("GitHub".into());
    g.invoke_save();
    assert_eq!(g.get_error(), "Each condition needs a value.");
    g.invoke_set_cond(0, 0, 4, "@github.com".into());
    g.invoke_save();
    assert_eq!(g.get_error(), "Choose at least one action.");
    g.set_act_read(true);
    g.invoke_save();
    assert!(!g.get_editing());
    assert_eq!(g.get_items().row_count(), 1);
    assert_eq!(
        g.get_items().row_data(0).unwrap().summary,
        "From ends with @github.com \u{b7} Mark as read"
    );

    // A second one with two conditions, the second removed again.
    g.invoke_add();
    g.set_name("Invoices".into());
    g.invoke_set_cond(0, 2, 0, "invoice".into());
    g.invoke_add_cond();
    assert_eq!(g.get_conds().row_count(), 2);
    g.invoke_set_cond(1, 3, 0, "".into());
    g.invoke_remove_cond(1);
    assert_eq!(g.get_conds().row_count(), 1);
    g.set_act_flag(true);
    g.invoke_save();
    assert!(!g.get_editing());

    let db = Db::open(&h.path).unwrap();
    let names = |db: &Db| -> Vec<String> {
        db.rules_for(account)
            .unwrap()
            .into_iter()
            .map(|r| r.name)
            .collect()
    };
    assert_eq!(names(&db), ["GitHub", "Invoices"]);
    let saved = &db.rules_for(account).unwrap()[1];
    assert_eq!(saved.conditions.len(), 1);
    assert!(saved.flag && !saved.mark_read);

    // Off, then down: both persist.
    g.invoke_toggle(0, false);
    assert!(!db.rules_for(account).unwrap()[0].enabled);
    assert!(!g.get_items().row_data(0).unwrap().enabled);
    g.invoke_move(0, false);
    assert_eq!(names(&db), ["Invoices", "GitHub"]);

    // Delete asks first.
    g.invoke_delete(1);
    assert!(ui.get_q_title().contains("GitHub"), "{}", ui.get_q_title());
    assert_eq!(names(&db).len(), 2);
    h.app(|a| a.answer(true));
    assert_eq!(names(&db), ["Invoices"]);
    assert_eq!(g.get_items().row_count(), 1);
}

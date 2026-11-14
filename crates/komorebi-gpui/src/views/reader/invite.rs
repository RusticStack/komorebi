//! The invite card (`komorebi::invite`): a meeting invitation in the open
//! message, drawn outside the page as an inset card: the title (struck
//! through when cancelled), the state line, when, who organizes it, where,
//! how it repeats and whether it is in the calendar; Accept, Tentative and
//! Decline (the answer given in ink); then Change, Join, Details, Remove
//! from calendar and Save .ics, as the Slint card offers them.

use std::rc::Rc;

use gpui::{
    App, ElementId, IntoElement, ParentElement, RenderOnce, Role, SharedString, Styled, Window,
    div, prelude::*, px,
};
use gpui_kit::TestSupportExt as _;
use komorebi::invite::{Card, Status, Tool};
use komorebi::mail::ical::Answer;
use komorebi_i18n::tr;

use crate::icons::Icon;
use crate::theme::{ActiveTokens as _, TextStyled as _, layout, radius, reader, space};
use crate::widgets::button::{Button, Variant};

/// What a press on the card asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Press {
    Answer(Answer),
    Tool(Tool),
}

type OnPress = Rc<dyn Fn(Press, &mut Window, &mut App)>;

/// The state line's words.
pub fn status_text(s: Status) -> String {
    match s {
        Status::Cancelled => tr!("Cancelled: this event will not take place."),
        Status::Accepted => tr!("You accepted."),
        Status::Tentative => tr!("You tentatively accepted."),
        Status::Declined => tr!("You declined."),
        Status::Over => tr!("This event is over."),
        Status::Updated => tr!("Updated invitation"),
    }
}

/// The card's lines under the title, one per fact.
pub fn lines(card: &Card) -> String {
    let mut lines = vec![card.when.clone()];
    if let Some(o) = &card.organizer {
        lines.push(tr!("Organizer: {0}", o));
    }
    if !card.location.is_empty() {
        lines.push(tr!("Where: {0}", card.location));
    }
    lines.push(card.repeats.clone());
    lines.join("\n").trim_end().to_string()
}

/// The card's title.
pub fn title(card: &Card) -> String {
    if card.title.is_empty() {
        tr!("(no title)")
    } else {
        card.title.clone()
    }
}

#[derive(IntoElement)]
pub struct InviteCard {
    card: Card,
    details: bool,
    on_press: OnPress,
}

impl InviteCard {
    /// `details`: the description is open.
    pub fn new(
        card: Card,
        details: bool,
        on_press: impl Fn(Press, &mut Window, &mut App) + 'static,
    ) -> Self {
        InviteCard {
            card,
            details,
            on_press: Rc::new(on_press),
        }
    }
}

fn answer_label(a: Answer) -> String {
    match a {
        Answer::Accepted => tr!("Accept"),
        Answer::Tentative => tr!("Tentative"),
        Answer::Declined => tr!("Decline"),
    }
}

impl RenderOnce for InviteCard {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = cx.tokens().clone();
        let c = t.color;
        let card = self.card;
        let title: SharedString = title(&card).into();
        let press = self.on_press;
        let answers = card.answers.then(|| {
            div().flex().flex_wrap().gap(px(space::SP_2)).children(
                [Answer::Accepted, Answer::Tentative, Answer::Declined].map(|a| {
                    let press = press.clone();
                    Button::new(ElementId::Name(
                        format!("invite-answer-{}", a as i32).into(),
                    ))
                    .variant(if card.answer == Some(a) {
                        Variant::Primary
                    } else {
                        Variant::OnInset
                    })
                    .label(answer_label(a))
                    .on_click(move |_, window, cx| press(Press::Answer(a), window, cx))
                }),
            )
        });
        // No calendar in 1.0: nothing offers to take the event out of one.
        let tools = card
            .tools
            .iter()
            .filter(|t| **t != Tool::Remove)
            .map(|&tool| {
                let (icon, label) = match tool {
                    Tool::Change => (Icon::Compose, tr!("Change")),
                    Tool::Join => (Icon::Open, tr!("Join")),
                    Tool::SaveIcs => (Icon::Save, tr!("Save .ics")),
                    Tool::Remove => unreachable!("filtered out above"),
                    Tool::Details if self.details => (Icon::ChevronUp, tr!("Details")),
                    Tool::Details => (Icon::ChevronDown, tr!("Details")),
                };
                let press = press.clone();
                Button::new(ElementId::Name(
                    format!("invite-tool-{}", tool as i32).into(),
                ))
                .variant(Variant::Subtle)
                .icon(icon)
                .label(label)
                .on_click(move |_, window, cx| press(Press::Tool(tool), window, cx))
            });
        let title_colour = if card.cancelled { c.ink_2 } else { c.ink };
        div()
            .id("invite-card")
            .role(Role::Group)
            .aria_label(title.clone())
            .w_full()
            .flex()
            .flex_col()
            .gap(px(space::SP_3))
            .p(px(space::SP_4))
            .rounded(px(radius::CARD))
            .bg(c.inset)
            .text_color(c.ink)
            .child(
                div()
                    .relative()
                    .typo(&t.text.subtitle)
                    .text_color(title_colour)
                    .child(title.clone())
                    // Struck through when cancelled (its first line).
                    .when(card.cancelled, |el| {
                        el.child(
                            div()
                                .absolute()
                                .left_0()
                                .right_0()
                                .top(px(reader::STRIKE_Y))
                                .h(px(layout::HAIRLINE))
                                .bg(c.ink_2),
                        )
                    }),
            )
            .when_some(card.status, |el, s| {
                el.child(
                    div()
                        .typo(&t.text.body_strong)
                        .text_color(c.ink_2)
                        .child(status_text(s)),
                )
            })
            .child(
                div()
                    .typo(&t.text.body)
                    .text_color(c.ink_2)
                    .child(lines(&card)),
            )
            .children(answers)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(space::SP_1))
                    .children(tools),
            )
            .when(self.details && !card.details.is_empty(), |el| {
                el.child(
                    div()
                        .typo(&t.text.body)
                        .text_color(c.ink_2)
                        .child(card.details.clone()),
                )
            })
            .test_support()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use komorebi::i18n::Lang;
    use komorebi_i18n::with_lang;

    fn card() -> Card {
        Card {
            title: "Planning".into(),
            when: "Thu 2099-10-08 14:00".into(),
            organizer: Some("Ana <ana@example.org>".into()),
            location: "Room 2".into(),
            repeats: String::new(),
            in_calendar: true,
            status: Some(Status::Updated),
            details: "Agenda".into(),
            cancelled: false,
            answers: true,
            answer: None,
            tools: vec![Tool::Details, Tool::SaveIcs],
        }
    }

    #[test]
    fn the_card_reads_as_in_the_slint_app() {
        with_lang(Lang::En, || {
            assert_eq!(
                lines(&card()),
                "Thu 2099-10-08 14:00\nOrganizer: Ana <ana@example.org>\nWhere: Room 2"
            );
            assert_eq!(status_text(Status::Updated), "Updated invitation");
            assert_eq!(
                title(&Card {
                    title: String::new(),
                    ..card()
                }),
                "(no title)"
            );
        });
        with_lang(Lang::PtPt, || {
            assert_eq!(status_text(Status::Accepted), "Aceitou.");
        });
    }
}

//! The onboarding options, each composed from the kit's pieces on the
//! full-window page.

use crate::desktop::i18n::tr;
use rune::{El, col, icon, row, text};
use rune_kit::{Button, ButtonVariant, glyphs};

use super::super::controls::{Paint, PushSize};
use super::landing::arrives;
use super::stories::{Flow, Onboard};

pub(super) fn view(p: Paint, s: &Onboard) -> El {
    let page = match s.flow {
        Flow::Versus => super::versus::view(p, s),
        Flow::FirstRun => super::first_run::view(p, s),
    };
    col()
        .key("onboarding")
        .w_full()
        .h_full()
        .fill(p.k.c.bg_deep)
        .children(page)
}

/// Primary onward button (`Continue →`), matching the tour's forward CTA.
pub(super) fn continue_button(p: Paint, s: &Onboard) -> El {
    let label = tr("Continue");
    let button = Button::new(label)
        .variant(ButtonVariant::Contrast)
        .label_with(move |font, ink| {
            row()
                .gap(6.0)
                .center_y()
                .child(text(label).key("label").font(font).color(ink))
                .child(icon(&glyphs::ARROW_RIGHT, 15.0).key("arrow").color(ink))
        });
    let button = if s.live.is_some() {
        button.on_press(super::live::press_continue)
    } else {
        button.on_press(Onboard::press_continue)
    };
    button.build_push(p.k).key("continue").transition(arrives())
}

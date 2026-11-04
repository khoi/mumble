//! Example macOS notifications to answer by voice, drawn strictly with
//! OpenCode theme tokens. Supports stacked sent bubbles above the input field
//! during the `"Without Hex"` double-send typo comedy!

use rune::{Color, El, Path, Shape, Spring, TextStyle, Transition, canvas, col, el, row, text};

use super::super::controls::Paint;
use super::super::lists::allowed;
use super::field_text;

pub(crate) const CARD_W: f32 = 404.0;
const CARD_PAD: f32 = 14.0;
const RADIUS: f32 = 16.0;
const ICON: f32 = 32.0;
const FIELD_PX: f32 = 12.0;
const FIELD_PY: f32 = 7.0;
pub(crate) const FIELD_FONT: TextStyle = TextStyle::new(13.0, 440.0);
pub(crate) const FIELD_LINE: f32 = 18.0;
const SENT_W: f32 = 58.0;
pub(crate) const REPLY_W: f32 = CARD_W - 2.0 * CARD_PAD - 2.0 * FIELD_PX - SENT_W;
const DRIFT: f32 = 2.5;
const DRIFT_SECS: f64 = 5.0;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum App {
    Messages,
    Slack,
}

impl App {
    fn name(self) -> &'static str {
        match self {
            Self::Messages => "Messages",
            Self::Slack => "Slack",
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Notice {
    pub(crate) key: &'static str,
    pub(crate) app: App,
    pub(crate) from: &'static str,
    pub(crate) place: Option<&'static str>,
    pub(crate) body: &'static str,
    pub(crate) field: &'static str,
}

pub(crate) const FRANK: Notice = Notice {
    key: "frank",
    app: App::Messages,
    from: "Frank",
    place: None,
    body: "your phone is on silent right",
    field: "iMessage",
};
pub(crate) const JAY: Notice = Notice {
    key: "jay",
    app: App::Messages,
    from: "Jay · CEO",
    place: None,
    body: "The investors are in the room. Are we ready to ship?",
    field: "iMessage",
};
pub(crate) const DAX: Notice = Notice {
    key: "dax",
    app: App::Slack,
    from: "Dax",
    place: Some("#opencode"),
    body: "can you look at the effect PR before 5",
    field: "Reply in thread",
};
pub(crate) const ADAM: Notice = Notice {
    key: "adam",
    app: App::Slack,
    from: "Adam",
    place: Some("#hex"),
    body: "hex just typed my standup into figma",
    field: "Reply in thread",
};

/// The card's reply field: the typed/spoken text, any sent bubbles, and
/// whether it is focused or already sent.
#[derive(Clone, Copy)]
pub(crate) struct Reply<'a> {
    pub(crate) sent_bubbles: &'a [String],
    pub(crate) text: &'a str,
    pub(crate) caret: Option<usize>,
    pub(crate) lines: usize,
    pub(crate) placeholder: bool,
    pub(crate) focus: bool,
    pub(crate) sent: bool,
}

/// Height of [`notice`] when its reply field has empty
/// `sent_bubbles` and `lines` wrapped lines.
pub(crate) fn lines_card_height(n: &Notice, lines: usize) -> f32 {
    // Measure how many lines `n.body` takes inside the top message column
    // (`CARD_W - 2*CARD_PAD - ICON - 10.0`).
    let k = rune_theme_opencode::kit(true);
    let body_w = CARD_W - 2.0 * CARD_PAD - ICON - 10.0;
    let (_, bh) = rune::text::measure(n.body, &[], k.ty.ui, Some(body_w));
    let top_h = (12.0 + 1.0 + 20.0 + 1.0 + bh).max(ICON);
    2.0 * CARD_PAD + top_h + 10.0 + 2.0 * FIELD_PY + lines as f32 * FIELD_LINE
}

pub(crate) fn notice(p: Paint, n: &Notice, reply: Reply<'_>, example: bool) -> El {
    let k = p.k;
    let head = row()
        .key("head")
        .gap(6.0)
        .center_y()
        .child(
            text(n.app.name())
                .key("app")
                .font(k.ty.small)
                .color(p.muted()),
        )
        .child(el().key("space").grow(1.0))
        .child(
            text(if example { "example" } else { "now" })
                .key("time")
                .font(k.ty.small)
                .color(p.faint()),
        );
    let title = match n.place {
        Some(place) => format!("{} in {place}", n.from),
        None => n.from.to_string(),
    };
    let message = col()
        .key("message")
        .grow(1.0)
        .gap(1.0)
        .child(head)
        .child(
            text(title)
                .key("title")
                .font(TextStyle {
                    weight: if k.dark { 560.0 } else { 600.0 },
                    ..k.ty.ui_medium
                })
                .color(p.text()),
        )
        .child(
            text(n.body)
                .key("body")
                .font(k.ty.ui)
                .color(p.text())
                .wrap()
                .max_lines(2),
        );

    let bubbles: &[String] = reply.sent_bubbles;

    let (fill, rim) = material(p);
    let mut card = col()
        .key(n.key)
        .w(CARD_W)
        .padding(CARD_PAD)
        .gap(10.0)
        .radius(RADIUS)
        .continuous_corners()
        .fill(fill)
        .backdrop_blur(24.0)
        .border(p.hair, rim)
        .motion(Spring::new(0.24, 0.0))
        .child(
            row()
                .key("top")
                .gap(10.0)
                .items_start()
                .child(app_icon(p, n.app))
                .child(message),
        );

    if !bubbles.is_empty() {
        card = card.child(sent_thread(p, bubbles));
    }

    // Remove the field in the same layout change as the second sent bubble,
    // not when Return lifts. Otherwise the card first makes room for both,
    // then shrinks while the departing field is already outside its new edge.
    let hide_field = bubbles.len() >= 2 && reply.text.is_empty();
    if !hide_field {
        card = card.child(field(p, n, reply));
    }

    k.elevate(card, &k.elevation.floating)
}

/// Right-aligned sent bubbles for the accidental double-send typos!
fn sent_thread(p: Paint, bubbles: &[String]) -> El {
    let k = p.k;
    let bubble_bg = rune_theme_opencode::scale("cobalt", k.dark, if k.dark { 4 } else { 3 });
    let bubble_rim = rune_theme_opencode::scale("cobalt", k.dark, if k.dark { 7 } else { 6 });
    let mut list = col().key("sent-thread").w_full().gap(5.0).items_end();
    for (i, msg) in bubbles.iter().enumerate() {
        list = list.child(
            row()
                .key(format!("sent-bubble-{i}"))
                .px(10.0)
                .py(4.0)
                .gap(6.0)
                .center_y()
                .radius(12.0)
                .fill(bubble_bg)
                .border(p.hair, bubble_rim)
                // The travel comes from the launch out of the field
                // (`stories::poll`); the bubble itself only fades up.
                .transition(Transition::opacity())
                .child(
                    text(msg.clone())
                        .key("msg")
                        .font(FIELD_FONT.lh(16.0))
                        .color(p.text()),
                )
                .child(text("Sent").key("tag").font(k.ty.small).color(p.muted())),
        );
    }
    list
}

fn material(p: Paint) -> (Color, Color) {
    let k = p.k;
    if k.dark {
        (k.c.surface_raised, k.c.border_weak)
    } else {
        (k.c.bg_base, k.c.border_weak)
    }
}

pub(crate) fn field_text_origin(card_bottom_left: [f32; 2], lines: usize) -> [f32; 2] {
    let [x, bottom] = card_bottom_left;
    [
        x + CARD_PAD + FIELD_PX,
        bottom - CARD_PAD - FIELD_PY - lines as f32 * FIELD_LINE,
    ]
}

fn app_icon(p: Paint, app: App) -> El {
    let dark = p.k.dark;
    let white = p.k.c.white;
    let squircle = el()
        .key("icon")
        .size(ICON, ICON)
        .radius(ICON * 0.225)
        .continuous_corners()
        .center();
    match app {
        App::Messages => {
            let top = rune_theme_opencode::scale("apple", dark, 9);
            let bot = rune_theme_opencode::scale("apple", dark, 10);
            squircle
                .fill(bot)
                .gradient(top, bot)
                .child(canvas(bubble(white)).key("bubble").size(ICON, ICON))
        }
        App::Slack => squircle
            .fill(rune_theme_opencode::scale("lilac", true, 3))
            .child(
                text("#")
                    .key("mark")
                    .font(TextStyle::new(19.0, 750.0).lh(22.0))
                    .color(white),
            ),
    }
}

fn bubble(color: Color) -> Shape {
    const K: f32 = 0.5523;
    let (cx, cy, rx, ry) = (16.0, 15.0, 10.2, 8.4);
    let oval = Path::new()
        .move_to([cx + rx, cy])
        .cubic_to(
            [cx + rx, cy + ry * K],
            [cx + rx * K, cy + ry],
            [cx, cy + ry],
        )
        .cubic_to(
            [cx - rx * K, cy + ry],
            [cx - rx, cy + ry * K],
            [cx - rx, cy],
        )
        .cubic_to(
            [cx - rx, cy - ry * K],
            [cx - rx * K, cy - ry],
            [cx, cy - ry],
        )
        .cubic_to(
            [cx + rx * K, cy - ry],
            [cx + rx, cy - ry * K],
            [cx + rx, cy],
        )
        .close();
    let tail = Path::new()
        .move_to([9.2, 19.3])
        .quad_to([9.4, 23.4], [6.4, 25.3])
        .quad_to([10.8, 25.1], [13.5, 22.2])
        .close();
    Shape::new()
        .view_box(ICON, ICON)
        .fill_path(&oval, color)
        .fill_path(&tail, color)
}

pub(crate) fn floating(card: El, now: Option<f64>, phase: f64) -> El {
    let Some(now) = now else {
        return card;
    };
    let turn = std::f64::consts::TAU * (now / DRIFT_SECS + phase);
    let y = -DRIFT * (0.5 - 0.5 * turn.cos()) as f32;
    card.offset(0.0, y).offset_motion(Spring::new(0.08, 0.0))
}

fn field(p: Paint, n: &Notice, reply: Reply<'_>) -> El {
    let k = p.k;
    let body = {
        let body = field_text::typed(p, "typed", reply.text, reply.caret, reply.lines);
        if reply.placeholder {
            body.child(placeholder(p, n.field).absolute(4.0, 0.0))
        } else {
            body
        }
    };
    let trailing = reply.sent.then(|| allowed(p, "sent", "Sent"));
    let field = row()
        .key("field")
        .px(FIELD_PX)
        .py(FIELD_PY)
        .gap(10.0)
        .radius(k.radius.lg)
        .fill(if k.dark { k.c.bg_deep } else { k.c.layer_01 })
        .border(
            p.hair,
            if reply.focus {
                k.c.border_focus
            } else {
                k.c.border_base
            },
        )
        .child(body)
        .children(trailing);
    field.items_end()
}

fn placeholder(p: Paint, copy: &str) -> El {
    text(copy)
        .key("placeholder")
        .font(FIELD_FONT.lh(FIELD_LINE))
        .color(p.faint())
        .grow(1.0)
}

//! The floating capsule. It is a Wayland layer surface that never takes keyboard focus and
//! lets clicks pass through, so pasting always lands in the app the user was in.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use gtk4::prelude::*;
use gtk4::{self as gtk, cairo, gdk, gio, glib};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

use crate::audio::Level;
use crate::pipeline::UiEvent;

const WIDTH: i32 = 216;
const HEIGHT: i32 = 40;
const BOTTOM_MARGIN: i32 = 64;
const BARS: usize = 22;
const BAR_WIDTH: f64 = 3.0;
const BAR_GAP: f64 = 3.0;
/// The waveform scrolls at a fixed rate whatever the monitor refresh rate is.
const SAMPLE_EVERY_US: i64 = 33_000;
const NOTICE_TIME: Duration = Duration::from_millis(2600);

// Pickforge brand tokens: surface-1, hairline-strong, ember, and text.
const SURFACE: (f64, f64, f64, f64) = (15.0 / 255.0, 15.0 / 255.0, 17.0 / 255.0, 0.94);
const HAIRLINE: (f64, f64, f64, f64) = (1.0, 1.0, 1.0, 0.14);
const EMBER: (f64, f64, f64) = (1.0, 122.0 / 255.0, 26.0 / 255.0);
const TEXT: (f64, f64, f64) = (242.0 / 255.0, 242.0 / 255.0, 243.0 / 255.0);

const CSS: &str = "
window.pickscribe { background: transparent; }
label.notice { color: #F2F2F3; font-family: 'Geist', 'Inter', sans-serif; font-size: 12px; }
";

#[derive(Debug, Clone, Copy, PartialEq)]
enum Mode {
    Hidden,
    Listening { raw: bool },
    Thinking { raw: bool },
    Notice,
}

struct View {
    mode: Mode,
    levels: VecDeque<f32>,
    now_us: i64,
    last_sample_us: i64,
    notice_id: u64,
}

impl Default for View {
    fn default() -> Self {
        Self {
            mode: Mode::Hidden,
            levels: VecDeque::from(vec![0.0; BARS]),
            now_us: 0,
            last_sample_us: 0,
            notice_id: 0,
        }
    }
}

#[derive(Clone)]
struct Widgets {
    window: gtk::ApplicationWindow,
    label: gtk::Label,
    view: Rc<RefCell<View>>,
}

pub fn run(events: async_channel::Receiver<UiEvent>, level: Level) {
    let app = gtk::Application::builder()
        .application_id("dev.pickforge.PickScribe")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.connect_activate(move |app| {
        // The capsule is hidden most of the time; keep the app alive without a visible window.
        std::mem::forget(app.hold());
        build(app, events.clone(), level.clone());
    });
    app.run_with_args::<&str>(&[]);
}

fn build(app: &gtk::Application, events: async_channel::Receiver<UiEvent>, level: Level) {
    install_css();
    let window = gtk::ApplicationWindow::new(app);
    window.add_css_class("pickscribe");
    window.set_default_size(WIDTH, HEIGHT);
    window.set_resizable(false);
    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    window.set_namespace(Some("pickscribe"));
    window.set_anchor(Edge::Bottom, true);
    window.set_margin(Edge::Bottom, BOTTOM_MARGIN);
    window.set_keyboard_mode(KeyboardMode::None);
    window.connect_realize(|window| {
        if let Some(surface) = window.surface() {
            surface.set_input_region(Some(&cairo::Region::create()));
        }
    });

    let view = Rc::new(RefCell::new(View::default()));
    let area = gtk::DrawingArea::builder()
        .content_width(WIDTH)
        .content_height(HEIGHT)
        .build();
    let draw_view = Rc::clone(&view);
    area.set_draw_func(move |_, cr, width, height| {
        draw(cr, f64::from(width), f64::from(height), &draw_view.borrow())
    });
    let tick_view = Rc::clone(&view);
    area.add_tick_callback(move |area, clock| {
        tick(&mut tick_view.borrow_mut(), clock.frame_time(), &level);
        area.queue_draw();
        glib::ControlFlow::Continue
    });

    let label = gtk::Label::builder()
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .visible(false)
        .build();
    label.add_css_class("notice");
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label.set_max_width_chars(30);
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&area));
    overlay.add_overlay(&label);
    window.set_child(Some(&overlay));

    let widgets = Widgets {
        window,
        label,
        view,
    };
    glib::spawn_future_local(async move {
        while let Ok(event) = events.recv().await {
            apply(&widgets, event);
        }
    });
}

fn install_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(CSS);
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

fn tick(view: &mut View, now_us: i64, level: &Level) {
    view.now_us = now_us;
    if matches!(view.mode, Mode::Listening { .. })
        && now_us - view.last_sample_us >= SAMPLE_EVERY_US
    {
        view.last_sample_us = now_us;
        view.levels.pop_front();
        view.levels
            .push_back(bar_height(f32::from_bits(level.load(Ordering::Relaxed))));
    }
}

/// Maps microphone RMS to 0..1. The curve lifts quiet speech so the bars react to it.
fn bar_height(rms: f32) -> f32 {
    (rms * 14.0).clamp(0.0, 1.0).powf(0.6)
}

fn apply(widgets: &Widgets, event: UiEvent) {
    // Release the borrow before touching widgets, which may draw synchronously.
    let notice = {
        let mut view = widgets.view.borrow_mut();
        match &event {
            UiEvent::Listening { raw } => {
                view.mode = Mode::Listening { raw: *raw };
                view.levels.iter_mut().for_each(|level| *level = 0.0);
            }
            UiEvent::Thinking { raw } => view.mode = Mode::Thinking { raw: *raw },
            UiEvent::Hidden => view.mode = Mode::Hidden,
            UiEvent::Notice(_) => {
                view.mode = Mode::Notice;
                view.notice_id += 1;
            }
        }
        view.notice_id
    };
    match event {
        UiEvent::Hidden => widgets.window.set_visible(false),
        UiEvent::Notice(text) => {
            widgets.label.set_text(&text);
            widgets.label.set_visible(true);
            widgets.window.set_visible(true);
            hide_notice_later(widgets.clone(), notice);
        }
        UiEvent::Listening { .. } | UiEvent::Thinking { .. } => {
            widgets.label.set_visible(false);
            widgets.window.set_visible(true);
        }
    }
}

fn hide_notice_later(widgets: Widgets, notice: u64) {
    glib::timeout_add_local_once(NOTICE_TIME, move || {
        let current = {
            let view = widgets.view.borrow();
            view.mode == Mode::Notice && view.notice_id == notice
        };
        if current {
            widgets.view.borrow_mut().mode = Mode::Hidden;
            widgets.window.set_visible(false);
        }
    });
}

fn draw(cr: &cairo::Context, width: f64, height: f64, view: &View) {
    capsule(cr, 0.5, 0.5, width - 1.0, height - 1.0);
    cr.set_source_rgba(SURFACE.0, SURFACE.1, SURFACE.2, SURFACE.3);
    let _ = cr.fill_preserve();
    cr.set_source_rgba(HAIRLINE.0, HAIRLINE.1, HAIRLINE.2, HAIRLINE.3);
    cr.set_line_width(1.0);
    let _ = cr.stroke();
    match view.mode {
        Mode::Listening { raw } => bars(cr, width, height, view.levels.iter().copied(), raw),
        Mode::Thinking { raw } => bars(cr, width, height, thinking_wave(view.now_us), raw),
        Mode::Hidden | Mode::Notice => {}
    }
}

fn capsule(cr: &cairo::Context, x: f64, y: f64, width: f64, height: f64) {
    let radius = height / 2.0;
    cr.new_sub_path();
    cr.arc(
        x + width - radius,
        y + radius,
        radius,
        -std::f64::consts::FRAC_PI_2,
        std::f64::consts::FRAC_PI_2,
    );
    cr.arc(
        x + radius,
        y + radius,
        radius,
        std::f64::consts::FRAC_PI_2,
        3.0 * std::f64::consts::FRAC_PI_2,
    );
    cr.close_path();
}

/// Ember bars mean the text will go through AI cleanup; off-white bars mean raw text.
fn bars(
    cr: &cairo::Context,
    width: f64,
    height: f64,
    levels: impl Iterator<Item = f32>,
    raw: bool,
) {
    let (r, g, b) = if raw { TEXT } else { EMBER };
    cr.set_source_rgb(r, g, b);
    cr.set_line_width(BAR_WIDTH);
    cr.set_line_cap(cairo::LineCap::Round);
    let total = BARS as f64 * (BAR_WIDTH + BAR_GAP) - BAR_GAP;
    let left = (width - total) / 2.0 + BAR_WIDTH / 2.0;
    let span = height - 18.0;
    for (index, level) in levels.enumerate() {
        let x = left + index as f64 * (BAR_WIDTH + BAR_GAP);
        let half = (f64::from(level) * span).max(1.0) / 2.0;
        cr.move_to(x, height / 2.0 - half);
        cr.line_to(x, height / 2.0 + half);
    }
    let _ = cr.stroke();
}

/// A pulse that sweeps across the bars while Whisper and the cleanup run.
fn thinking_wave(now_us: i64) -> impl Iterator<Item = f32> {
    let seconds = now_us as f64 / 1_000_000.0;
    let center = (BARS as f64 - 1.0) * (0.5 - 0.5 * (seconds * 2.4).cos());
    (0..BARS).map(move |index| {
        let distance = index as f64 - center;
        (0.12 + 0.55 * (-(distance * distance) / 8.0).exp()) as f32
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bar_height_is_bounded_and_rises_with_level() {
        assert_eq!(bar_height(0.0), 0.0);
        assert!(bar_height(0.01) < bar_height(0.05));
        assert_eq!(bar_height(5.0), 1.0);
    }

    #[test]
    fn thinking_wave_stays_in_range() {
        for now in [0, 400_000, 1_300_000, 9_999_999] {
            let wave: Vec<f32> = thinking_wave(now).collect();
            assert_eq!(wave.len(), BARS);
            assert!(wave.iter().all(|level| (0.1..=0.7).contains(level)));
        }
    }
}

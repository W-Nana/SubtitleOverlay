use std::{cell::RefCell, env, f64::consts::PI, rc::Rc, sync::Arc, time::Duration};

use gtk::{
    cairo::{Context, Operator},
    gdk,
    glib::{self, ControlFlow, Propagation},
    pango,
    prelude::*,
};

use crate::{OverlayBounds, Settings, SubtitleEntry};

const MIN_WIDTH: u32 = 280;
const MIN_HEIGHT: u32 = 88;
const RESIZE_EDGE: f64 = 8.0;
const RESIZE_CORNER: f64 = 18.0;

type BoundsCallback = Arc<dyn Fn(OverlayBounds) + 'static>;

#[derive(Clone)]
pub struct NativeOverlayHandle {
    sender: glib::Sender<NativeOverlayMessage>,
}

impl NativeOverlayHandle {
    pub fn update_settings(&self, settings: Settings) {
        let _ = self
            .sender
            .send(NativeOverlayMessage::UpdateSettings(settings));
    }

    pub fn update_subtitles(&self, subtitles: Vec<SubtitleEntry>) {
        let _ = self
            .sender
            .send(NativeOverlayMessage::UpdateSubtitles(subtitles));
    }

    pub fn apply_bounds(&self, bounds: OverlayBounds) {
        let _ = self.sender.send(NativeOverlayMessage::ApplyBounds(bounds));
    }

    pub fn close(&self) {
        let _ = self.sender.send(NativeOverlayMessage::Close);
    }
}

enum NativeOverlayMessage {
    UpdateSettings(Settings),
    UpdateSubtitles(Vec<SubtitleEntry>),
    ApplyBounds(OverlayBounds),
    Close,
}

#[derive(Clone)]
struct OverlayModel {
    settings: Settings,
    subtitles: Vec<SubtitleEntry>,
    resizing: bool,
}

pub fn should_use_native_overlay() -> bool {
    let session_type = env::var("XDG_SESSION_TYPE")
        .unwrap_or_default()
        .to_ascii_lowercase();

    session_type == "x11"
        || (session_type.is_empty()
            && env::var_os("DISPLAY").is_some()
            && env::var_os("WAYLAND_DISPLAY").is_none())
}

#[allow(deprecated)]
pub fn spawn(
    settings: Settings,
    subtitles: Vec<SubtitleEntry>,
    on_bounds_changed: impl Fn(OverlayBounds) + 'static,
) -> NativeOverlayHandle {
    let (sender, receiver) = glib::MainContext::channel(glib::Priority::default());
    let overlay = NativeOverlay::new(settings, subtitles, Arc::new(on_bounds_changed));

    receiver.attach(None, move |message| {
        match message {
            NativeOverlayMessage::UpdateSettings(settings) => overlay.update_settings(settings),
            NativeOverlayMessage::UpdateSubtitles(subtitles) => overlay.update_subtitles(subtitles),
            NativeOverlayMessage::ApplyBounds(bounds) => overlay.apply_bounds(bounds),
            NativeOverlayMessage::Close => {
                overlay.close();
                return ControlFlow::Break;
            }
        }

        ControlFlow::Continue
    });

    NativeOverlayHandle { sender }
}

struct NativeOverlay {
    window: gtk::Window,
    canvas: gtk::DrawingArea,
    model: Rc<RefCell<OverlayModel>>,
    resize_frame_timer: Rc<RefCell<Option<glib::SourceId>>>,
    applying_bounds: Rc<RefCell<bool>>,
}

impl NativeOverlay {
    fn new(
        settings: Settings,
        subtitles: Vec<SubtitleEntry>,
        on_bounds_changed: BoundsCallback,
    ) -> Self {
        let model = Rc::new(RefCell::new(OverlayModel {
            settings: settings.clone(),
            subtitles,
            resizing: false,
        }));

        let window = gtk::Window::new(gtk::WindowType::Toplevel);
        let canvas = gtk::DrawingArea::new();
        let resize_frame_timer = Rc::new(RefCell::new(None));
        let applying_bounds = Rc::new(RefCell::new(false));

        prepare_transparent_window(&window, &canvas);
        configure_window_hints(&window);

        let bounds = normalized_bounds(&settings.overlay_bounds);
        canvas.set_size_request(MIN_WIDTH as i32, MIN_HEIGHT as i32);
        window.set_default_size(bounds.width as i32, bounds.height as i32);
        window.resize(bounds.width as i32, bounds.height as i32);
        window.add(&canvas);

        connect_draw(&canvas, model.clone());
        connect_interactions(&window, &canvas, model.clone(), resize_frame_timer.clone());
        connect_bounds_persistence(&window, applying_bounds.clone(), on_bounds_changed);

        window.show_all();
        apply_initial_position(&window, &bounds);
        window.set_keep_above(true);

        Self {
            window,
            canvas,
            model,
            resize_frame_timer,
            applying_bounds,
        }
    }

    fn update_settings(&self, settings: Settings) {
        let bounds = normalized_bounds(&settings.overlay_bounds);
        self.model.borrow_mut().settings = settings;
        self.canvas.queue_draw();
        self.apply_bounds(bounds);
    }

    fn update_subtitles(&self, subtitles: Vec<SubtitleEntry>) {
        self.model.borrow_mut().subtitles = subtitles;
        self.canvas.queue_draw();
    }

    fn apply_bounds(&self, bounds: OverlayBounds) {
        let bounds = normalized_bounds(&bounds);
        *self.applying_bounds.borrow_mut() = true;
        self.window
            .resize(bounds.width as i32, bounds.height as i32);
        if let (Some(x), Some(y)) = (bounds.x, bounds.y) {
            self.window.move_(x, y);
        }

        let applying_bounds = self.applying_bounds.clone();
        glib::idle_add_local_once(move || {
            *applying_bounds.borrow_mut() = false;
        });
    }

    fn close(&self) {
        if let Some(timer) = self.resize_frame_timer.borrow_mut().take() {
            timer.remove();
        }
        self.window.close();
    }
}

fn prepare_transparent_window(window: &gtk::Window, canvas: &gtk::DrawingArea) {
    window.set_app_paintable(true);
    canvas.set_app_paintable(true);
    window.set_widget_name("subtitle-native-overlay-window");
    canvas.set_widget_name("subtitle-native-overlay-canvas");

    if let Some(screen) = gtk::prelude::WidgetExt::screen(window) {
        if let Some(visual) = screen.rgba_visual() {
            window.set_visual(Some(&visual));
        }
    }

    let css_provider = gtk::CssProvider::new();
    if let Err(err) = css_provider.load_from_data(
        b"#subtitle-native-overlay-window,
          #subtitle-native-overlay-canvas {
            background: transparent;
            background-color: transparent;
            background-image: none;
            border: 0;
            box-shadow: none;
          }",
    ) {
        eprintln!("failed to load native overlay GTK CSS: {err}");
    }

    window
        .style_context()
        .add_provider(&css_provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    canvas
        .style_context()
        .add_provider(&css_provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
}

fn configure_window_hints(window: &gtk::Window) {
    window.set_title("SubtitleOverlay");
    window.set_decorated(false);
    window.set_resizable(true);
    window.set_keep_above(true);
    window.set_skip_taskbar_hint(true);
    window.set_skip_pager_hint(true);
    window.set_focus_on_map(false);
    window.set_accept_focus(false);
    window.set_type_hint(gdk::WindowTypeHint::Utility);
    window.add_events(
        gdk::EventMask::BUTTON_PRESS_MASK
            | gdk::EventMask::BUTTON_RELEASE_MASK
            | gdk::EventMask::POINTER_MOTION_MASK
            | gdk::EventMask::STRUCTURE_MASK,
    );
}

fn connect_draw(canvas: &gtk::DrawingArea, model: Rc<RefCell<OverlayModel>>) {
    canvas.connect_draw(move |widget, cr| {
        draw_overlay(widget, cr, &model.borrow());
        Propagation::Stop
    });
}

fn connect_interactions(
    window: &gtk::Window,
    canvas: &gtk::DrawingArea,
    model: Rc<RefCell<OverlayModel>>,
    resize_frame_timer: Rc<RefCell<Option<glib::SourceId>>>,
) {
    canvas.add_events(
        gdk::EventMask::BUTTON_PRESS_MASK
            | gdk::EventMask::BUTTON_RELEASE_MASK
            | gdk::EventMask::POINTER_MOTION_MASK,
    );

    let window_for_press = window.clone();
    let canvas_for_press = canvas.clone();
    let model_for_press = model.clone();
    let timer_for_press = resize_frame_timer.clone();
    canvas.connect_button_press_event(move |widget, event| {
        if event.button() != 1 {
            return Propagation::Proceed;
        }

        let (root_x, root_y) = event.root();
        let (x, y) = event.position();
        let hit = hit_test(widget.allocated_width(), widget.allocated_height(), x, y);

        match hit {
            HitTarget::Resize(edge) => {
                show_resize_frame(
                    &canvas_for_press,
                    model_for_press.clone(),
                    timer_for_press.clone(),
                    Duration::from_millis(900),
                );
                window_for_press.begin_resize_drag(
                    edge,
                    event.button() as i32,
                    root_x.round() as i32,
                    root_y.round() as i32,
                    event.time(),
                );
            }
            HitTarget::Move => {
                window_for_press.begin_move_drag(
                    event.button() as i32,
                    root_x.round() as i32,
                    root_y.round() as i32,
                    event.time(),
                );
            }
        }

        Propagation::Stop
    });

    let canvas_for_release = canvas.clone();
    let model_for_release = model.clone();
    let timer_for_release = resize_frame_timer.clone();
    canvas.connect_button_release_event(move |_, _| {
        show_resize_frame(
            &canvas_for_release,
            model_for_release.clone(),
            timer_for_release.clone(),
            Duration::from_millis(360),
        );
        Propagation::Proceed
    });

    canvas.connect_motion_notify_event(move |widget, event| {
        let (x, y) = event.position();
        update_cursor(
            widget,
            hit_test(widget.allocated_width(), widget.allocated_height(), x, y),
        );
        Propagation::Proceed
    });
}

fn connect_bounds_persistence(
    window: &gtk::Window,
    applying_bounds: Rc<RefCell<bool>>,
    on_bounds_changed: BoundsCallback,
) {
    let pending_bounds = Rc::new(RefCell::new(None::<OverlayBounds>));
    let save_timer = Rc::new(RefCell::new(None::<glib::SourceId>));

    window.connect_configure_event(move |_, event| {
        if *applying_bounds.borrow() {
            return false;
        }

        let (x, y) = event.position();
        let (width, height) = event.size();
        *pending_bounds.borrow_mut() = Some(OverlayBounds {
            x: Some(x),
            y: Some(y),
            width,
            height,
        });

        if let Some(timer) = save_timer.borrow_mut().take() {
            timer.remove();
        }

        let pending_bounds_for_timer = pending_bounds.clone();
        let save_timer_for_timer = save_timer.clone();
        let callback = on_bounds_changed.clone();
        *save_timer.borrow_mut() = Some(glib::timeout_add_local_once(
            Duration::from_millis(250),
            move || {
                if let Some(bounds) = pending_bounds_for_timer.borrow_mut().take() {
                    callback(bounds);
                }
                save_timer_for_timer.borrow_mut().take();
            },
        ));

        false
    });
}

fn draw_overlay(widget: &gtk::DrawingArea, cr: &Context, model: &OverlayModel) {
    let width = widget.allocated_width().max(1) as f64;
    let height = widget.allocated_height().max(1) as f64;
    let padding_x = 18.0;
    let padding_y = 14.0;
    let content_width = (width - padding_x * 2.0).max(1.0);
    let content_height = (height - padding_y * 2.0).max(1.0);

    clear(cr);
    draw_background(cr, &model.settings, width, height);

    cr.save().ok();
    rounded_rectangle(cr, 0.0, 0.0, width, height, 8.0);
    cr.clip();

    if model.subtitles.is_empty() {
        draw_waiting_text(
            widget,
            cr,
            &model.settings,
            padding_x,
            padding_y,
            content_width,
            content_height,
        );
    } else {
        draw_subtitles(
            widget,
            cr,
            model,
            padding_x,
            padding_y,
            content_width,
            content_height,
        );
    }

    cr.restore().ok();

    draw_resize_marker(cr, width, height);

    if model.resizing {
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.42);
        cr.set_line_width(1.0);
        rounded_rectangle(cr, 0.5, 0.5, width - 1.0, height - 1.0, 8.0);
        cr.stroke().ok();
    }
}

fn draw_resize_marker(cr: &Context, width: f64, height: f64) {
    cr.set_source_rgba(1.0, 1.0, 1.0, 0.42);
    cr.set_line_width(2.0);
    cr.move_to(width - 12.0, height - 4.0);
    cr.line_to(width - 4.0, height - 4.0);
    cr.line_to(width - 4.0, height - 12.0);
    cr.stroke().ok();
}

fn clear(cr: &Context) {
    cr.save().ok();
    cr.set_operator(Operator::Source);
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.0);
    cr.paint().ok();
    cr.restore().ok();
}

fn draw_background(cr: &Context, settings: &Settings, width: f64, height: f64) {
    let rgb = parse_hex_color(&settings.background_color, (0.0, 0.0, 0.0));
    cr.set_source_rgba(
        rgb.0,
        rgb.1,
        rgb.2,
        f64::from(settings.background_opacity.min(100)) / 100.0,
    );
    rounded_rectangle(cr, 0.0, 0.0, width, height, 8.0);
    cr.fill().ok();
}

fn draw_waiting_text(
    widget: &gtk::DrawingArea,
    cr: &Context,
    settings: &Settings,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) {
    let layout = create_layout(
        widget,
        "等待字幕串流...",
        &settings.font_family,
        settings.font_size,
        pango::Weight::Normal,
        width,
    );
    let (_, text_height) = layout.pixel_size();
    let text_y = y + ((height - f64::from(text_height)).max(0.0) / 2.0);
    draw_layout_with_shadow(cr, &layout, x, text_y, (1.0, 1.0, 1.0, 0.62));
}

fn draw_subtitles(
    widget: &gtk::DrawingArea,
    cr: &Context,
    model: &OverlayModel,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) {
    let mut rows = Vec::new();

    for (index, entry) in model.subtitles.iter().enumerate() {
        if !entry.original.is_empty() {
            rows.push(TextRow::text(
                entry.original.clone(),
                model.settings.original_text_color.clone(),
                pango::Weight::Normal,
            ));
        }
        if !entry.translated.is_empty() {
            rows.push(TextRow::text(
                entry.translated.clone(),
                model.settings.translated_text_color.clone(),
                pango::Weight::Semibold,
            ));
        }
        if index < model.subtitles.len() - 1 {
            rows.push(TextRow::divider());
        }
    }

    let measured_rows = rows
        .into_iter()
        .map(|row| {
            row.measure(
                widget,
                &model.settings.font_family,
                model.settings.font_size,
                width,
            )
        })
        .collect::<Vec<_>>();
    let total_height = measured_rows.iter().map(|row| row.height).sum::<f64>();
    let mut cursor_y = y - (total_height - height).max(0.0);

    for row in measured_rows {
        match row.kind {
            MeasuredRowKind::Text { layout, color } => {
                draw_layout_with_shadow(cr, &layout, x, cursor_y, color);
            }
            MeasuredRowKind::Divider => {
                cr.set_source_rgba(1.0, 1.0, 1.0, 0.2);
                cr.rectangle(x, cursor_y + 3.0, width, 1.0);
                cr.fill().ok();
            }
        }
        cursor_y += row.height;
    }
}

fn create_layout(
    widget: &gtk::DrawingArea,
    text: &str,
    font_family: &str,
    font_size: f32,
    weight: pango::Weight,
    width: f64,
) -> pango::Layout {
    let layout = widget.create_pango_layout(Some(text));
    let mut description = pango::FontDescription::from_string(font_family);
    description.set_size((font_size * pango::SCALE as f32).round() as i32);
    description.set_weight(weight);
    layout.set_font_description(Some(&description));
    layout.set_width((width * f64::from(pango::SCALE)).round() as i32);
    layout.set_wrap(pango::WrapMode::WordChar);
    layout
}

fn draw_layout_with_shadow(
    cr: &Context,
    layout: &pango::Layout,
    x: f64,
    y: f64,
    color: (f64, f64, f64, f64),
) {
    cr.save().ok();
    cr.translate(x + 1.0, y + 1.0);
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.86);
    pangocairo::functions::show_layout(cr, layout);
    cr.restore().ok();

    cr.save().ok();
    cr.translate(x, y);
    cr.set_source_rgba(color.0, color.1, color.2, color.3);
    pangocairo::functions::show_layout(cr, layout);
    cr.restore().ok();
}

fn rounded_rectangle(cr: &Context, x: f64, y: f64, width: f64, height: f64, radius: f64) {
    let radius = radius.min(width / 2.0).min(height / 2.0);
    cr.new_sub_path();
    cr.arc(x + width - radius, y + radius, radius, -PI / 2.0, 0.0);
    cr.arc(
        x + width - radius,
        y + height - radius,
        radius,
        0.0,
        PI / 2.0,
    );
    cr.arc(x + radius, y + height - radius, radius, PI / 2.0, PI);
    cr.arc(x + radius, y + radius, radius, PI, PI * 1.5);
    cr.close_path();
}

struct TextRow {
    kind: TextRowKind,
}

enum TextRowKind {
    Text {
        text: String,
        color: String,
        weight: pango::Weight,
    },
    Divider,
}

impl TextRow {
    fn text(text: String, color: String, weight: pango::Weight) -> Self {
        Self {
            kind: TextRowKind::Text {
                text,
                color,
                weight,
            },
        }
    }

    fn divider() -> Self {
        Self {
            kind: TextRowKind::Divider,
        }
    }

    fn measure(
        self,
        widget: &gtk::DrawingArea,
        font_family: &str,
        font_size: f32,
        width: f64,
    ) -> MeasuredRow {
        match self.kind {
            TextRowKind::Text {
                text,
                color,
                weight,
            } => {
                let layout = create_layout(widget, &text, font_family, font_size, weight, width);
                let (_, height) = layout.pixel_size();
                MeasuredRow {
                    height: f64::from(height) + 4.0,
                    kind: MeasuredRowKind::Text {
                        layout,
                        color: rgba_from_hex(&color, 1.0),
                    },
                }
            }
            TextRowKind::Divider => MeasuredRow {
                height: 7.0,
                kind: MeasuredRowKind::Divider,
            },
        }
    }
}

struct MeasuredRow {
    height: f64,
    kind: MeasuredRowKind,
}

enum MeasuredRowKind {
    Text {
        layout: pango::Layout,
        color: (f64, f64, f64, f64),
    },
    Divider,
}

#[derive(Clone, Copy)]
enum HitTarget {
    Move,
    Resize(gdk::WindowEdge),
}

fn hit_test(width: i32, height: i32, x: f64, y: f64) -> HitTarget {
    let width = f64::from(width.max(1));
    let height = f64::from(height.max(1));
    let on_right = x >= width - RESIZE_EDGE;
    let on_bottom = y >= height - RESIZE_EDGE;
    let on_corner = x >= width - RESIZE_CORNER && y >= height - RESIZE_CORNER;

    if on_corner {
        HitTarget::Resize(gdk::WindowEdge::SouthEast)
    } else if on_right {
        HitTarget::Resize(gdk::WindowEdge::East)
    } else if on_bottom {
        HitTarget::Resize(gdk::WindowEdge::South)
    } else {
        HitTarget::Move
    }
}

fn update_cursor(widget: &gtk::DrawingArea, hit: HitTarget) {
    let Some(window) = widget.window() else {
        return;
    };
    let Some(display) = gdk::Display::default() else {
        return;
    };

    let cursor_name = match hit {
        HitTarget::Move => None,
        HitTarget::Resize(gdk::WindowEdge::East) => Some("ew-resize"),
        HitTarget::Resize(gdk::WindowEdge::South) => Some("ns-resize"),
        HitTarget::Resize(gdk::WindowEdge::SouthEast) => Some("nwse-resize"),
        HitTarget::Resize(_) => None,
    };
    let cursor = cursor_name.and_then(|name| gdk::Cursor::from_name(&display, name));
    window.set_cursor(cursor.as_ref());
}

fn show_resize_frame(
    canvas: &gtk::DrawingArea,
    model: Rc<RefCell<OverlayModel>>,
    resize_frame_timer: Rc<RefCell<Option<glib::SourceId>>>,
    duration: Duration,
) {
    if let Some(timer) = resize_frame_timer.borrow_mut().take() {
        timer.remove();
    }

    model.borrow_mut().resizing = true;
    canvas.queue_draw();

    let canvas_for_timer = canvas.clone();
    let model_for_timer = model.clone();
    let timer_cell_for_timer = resize_frame_timer.clone();
    *resize_frame_timer.borrow_mut() = Some(glib::timeout_add_local_once(duration, move || {
        model_for_timer.borrow_mut().resizing = false;
        canvas_for_timer.queue_draw();
        timer_cell_for_timer.borrow_mut().take();
    }));
}

fn apply_initial_position(window: &gtk::Window, bounds: &OverlayBounds) {
    if let (Some(x), Some(y)) = (bounds.x, bounds.y) {
        window.move_(x, y);
        return;
    }

    let Some(display) = gdk::Display::default() else {
        return;
    };
    let Some(monitor) = display.primary_monitor().or_else(|| display.monitor(0)) else {
        return;
    };
    let area = monitor.workarea();
    let width = bounds.width as i32;
    let height = bounds.height as i32;
    let x = area.x() + ((area.width() - width).max(0) / 2);
    let y = area.y() + (area.height() - height - 72).max(0);
    window.move_(x, y);
}

fn normalized_bounds(bounds: &OverlayBounds) -> OverlayBounds {
    OverlayBounds {
        x: bounds.x,
        y: bounds.y,
        width: bounds.width.max(MIN_WIDTH),
        height: bounds.height.max(MIN_HEIGHT),
    }
}

fn rgba_from_hex(value: &str, alpha: f64) -> (f64, f64, f64, f64) {
    let (r, g, b) = parse_hex_color(value, (1.0, 1.0, 1.0));
    (r, g, b, alpha)
}

fn parse_hex_color(value: &str, fallback: (f64, f64, f64)) -> (f64, f64, f64) {
    let hex = value.trim().strip_prefix('#').unwrap_or(value.trim());
    if hex.len() != 6 {
        return fallback;
    }

    let Some(r) = parse_hex_component(&hex[0..2]) else {
        return fallback;
    };
    let Some(g) = parse_hex_component(&hex[2..4]) else {
        return fallback;
    };
    let Some(b) = parse_hex_component(&hex[4..6]) else {
        return fallback;
    };

    (r, g, b)
}

fn parse_hex_component(value: &str) -> Option<f64> {
    u8::from_str_radix(value, 16)
        .ok()
        .map(|component| f64::from(component) / 255.0)
}

//! The switcher overlay as plain AppKit views: a non-activating panel holding
//! a glass card, drawn by the window server. No Metal runs in this process.
//! A click on a row presses that row's key, so the engine decides a click
//! exactly as it decides the keystroke.

use std::cell::{Cell, RefCell};
use std::ptr::NonNull;
use std::rc::Rc;
use std::time::Duration;

use block2::{RcBlock, StackBlock};
use mojito_core::PhysicalKey;
use mojito_core::geom::{self, Rect};
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2::{AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSAnimatablePropertyContainer, NSAnimationContext, NSAppearanceCustomization,
    NSBackingStoreType, NSColor, NSEvent, NSEventMask, NSFont, NSImage, NSImageView,
    NSLineBreakMode, NSPanel, NSScreen, NSTextAlignment, NSTextField, NSTrackingArea,
    NSTrackingAreaOptions, NSView, NSVisualEffectBlendingMode, NSVisualEffectMaterial,
    NSVisualEffectState, NSVisualEffectView, NSWindowAnimationBehavior, NSWindowCollectionBehavior,
    NSWindowStyleMask,
};
use objc2_core_foundation::CFRetained;
use objc2_core_graphics::{CGColor, CGImage};
use objc2_foundation::{NSPoint, NSPointInRect, NSRect, NSSize, NSString};

const CARD_W: f64 = 380.0;
const CARD_RADIUS: f64 = 22.0;
const HEADER_H: f64 = 34.0;
const ROW_H: f64 = 36.0;
const ICON: f64 = 24.0;
const ROW_GAP: f64 = 2.0;
const PAD: f64 = 12.0;
const ROW_INSET: f64 = 10.0;
const ROW_RADIUS: f64 = 8.0;
const MAX_ROWS: usize = 12;
const STATUS_WINDOW_LEVEL: isize = 25;
const SEMIBOLD: f64 = 0.3;
const REGULAR: f64 = 0.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Pinned,
    Live,
}

/// Display-ready row: the badge is already the glyph of its key, and a
/// missing icon renders as the name's monogram. `key` is what a click on the
/// row presses; a row without one is not clickable.
pub struct Row {
    pub key: Option<PhysicalKey>,
    pub badge: Option<char>,
    pub name: String,
    pub windows: Option<u8>,
    pub icon: Option<CFRetained<CGImage>>,
}

pub struct Switcher {
    panel: Retained<NSPanel>,
    root: Retained<NSView>,
    monitor: Option<Retained<AnyObject>>,
    generation: Rc<Cell<u64>>,
    mtm: MainThreadMarker,
}

impl Switcher {
    pub fn new(mtm: MainThreadMarker) -> Self {
        let frame = NSRect::new(NSPoint::ZERO, NSSize::new(CARD_W, card_height(0)));
        let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
            NSPanel::alloc(mtm),
            frame,
            NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
            NSBackingStoreType::Buffered,
            true,
        );
        panel.setOpaque(false);
        panel.setBackgroundColor(Some(&NSColor::clearColor()));
        panel.setHasShadow(true);
        panel.setLevel(STATUS_WINDOW_LEVEL);
        panel.setHidesOnDeactivate(false);
        panel.setAnimationBehavior(NSWindowAnimationBehavior::None);
        panel.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::FullScreenAuxiliary
                | NSWindowCollectionBehavior::Stationary
                | NSWindowCollectionBehavior::IgnoresCycle,
        );
        let root = NSView::initWithFrame(NSView::alloc(mtm), frame);
        panel.setContentView(Some(&root));
        Self {
            panel,
            root,
            monitor: None,
            generation: Rc::new(Cell::new(0)),
            mtm,
        }
    }

    pub fn show(&mut self, mode: Mode, rows: &[Row]) {
        self.advance();
        clear(&self.root);
        let size = (CARD_W, card_height(rows.len()));
        self.panel
            .setFrame_display(placement(size, self.mtm), false);
        let built = RefCell::new(None);
        let build = StackBlock::new(|| {
            *built.borrow_mut() = Some(card(mode, rows, size.1, self.mtm));
        });
        self.panel
            .effectiveAppearance()
            .performAsCurrentDrawingAppearance(&build);
        if let Some(card) = built.into_inner() {
            self.root.addSubview(&card);
        }
        self.panel.setAlphaValue(1.0);
        self.panel.orderFrontRegardless();
        self.panel.invalidateShadow();
        self.watch_clicks();
    }

    pub fn hide(&mut self) {
        self.advance();
        self.unwatch_clicks();
        release(&self.panel, &self.root);
    }

    pub fn fade_out(&mut self, duration: Duration) {
        let generation = self.advance();
        self.unwatch_clicks();
        let fading = self.panel.clone();
        let changes = RcBlock::new(move |context: NonNull<NSAnimationContext>| {
            unsafe { context.as_ref() }.setDuration(duration.as_secs_f64());
            fading.animator().setAlphaValue(0.0);
        });
        let (panel, root, current) = (
            self.panel.clone(),
            self.root.clone(),
            self.generation.clone(),
        );
        let done = RcBlock::new(move || {
            if current.get() == generation {
                release(&panel, &root);
            }
        });
        NSAnimationContext::runAnimationGroup_completionHandler(&changes, Some(&done));
    }

    fn advance(&self) -> u64 {
        let next = self.generation.get() + 1;
        self.generation.set(next);
        next
    }

    fn watch_clicks(&mut self) {
        if self.monitor.is_some() {
            return;
        }
        let handler = RcBlock::new(|_: NonNull<NSEvent>| crate::synth::post_control());
        self.monitor = NSEvent::addGlobalMonitorForEventsMatchingMask_handler(
            NSEventMask::LeftMouseDown | NSEventMask::RightMouseDown | NSEventMask::OtherMouseDown,
            &handler,
        );
    }

    fn unwatch_clicks(&mut self) {
        if let Some(monitor) = self.monitor.take() {
            unsafe { NSEvent::removeMonitor(&monitor) };
        }
    }
}

fn release(panel: &NSPanel, root: &NSView) {
    panel.orderOut(None);
    clear(root);
}

fn clear(view: &NSView) {
    for subview in view.subviews().iter() {
        subview.removeFromSuperview();
    }
}

fn card_height(rows: usize) -> f64 {
    let shown = rows.min(MAX_ROWS);
    let gaps = shown.saturating_sub(1) as f64 * ROW_GAP;
    let footer = if rows > MAX_ROWS { ROW_H } else { 0.0 };
    let height = HEADER_H + PAD + shown as f64 * ROW_H + gaps + footer + PAD;
    height.max(HEADER_H + PAD + ROW_H + PAD)
}

fn placement(size: (f64, f64), mtm: MainThreadMarker) -> NSRect {
    let screens: Vec<Rect> = NSScreen::screens(mtm)
        .iter()
        .map(|screen| {
            let frame = screen.frame();
            Rect {
                x: frame.origin.x,
                y: frame.origin.y,
                w: frame.size.width,
                h: frame.size.height,
            }
        })
        .collect();
    let rect = if screens.is_empty() {
        Rect {
            x: 0.0,
            y: 0.0,
            w: size.0,
            h: size.1,
        }
    } else {
        let mouse = NSEvent::mouseLocation();
        let at = geom::display_containing((mouse.x, mouse.y), &screens);
        geom::centered_rect(size, screens[at])
    };
    NSRect::new(NSPoint::new(rect.x, rect.y), NSSize::new(rect.w, rect.h))
}

fn card(mode: Mode, rows: &[Row], height: f64, mtm: MainThreadMarker) -> Retained<NSView> {
    let bounds = NSRect::new(NSPoint::ZERO, NSSize::new(CARD_W, height));
    let content = NSView::initWithFrame(NSView::alloc(mtm), bounds);
    let painter = Painter::new(&content, height, mtm);
    painter.header(mode);
    for (index, row) in rows.iter().take(MAX_ROWS).enumerate() {
        painter.row(index, row);
    }
    if rows.is_empty() {
        painter.note(0, "No apps pinned yet");
    } else if rows.len() > MAX_ROWS {
        painter.note(MAX_ROWS, &format!("+{} more", rows.len() - MAX_ROWS));
    }
    glass(bounds, &content, mtm)
}

fn glass(bounds: NSRect, content: &NSView, mtm: MainThreadMarker) -> Retained<NSView> {
    if let Some(class) = AnyClass::get(c"NSGlassEffectView") {
        let view = unsafe {
            let allocated: *mut AnyObject = msg_send![class, alloc];
            let view: *mut NSView = msg_send![allocated, initWithFrame: bounds];
            Retained::from_raw(view)
        };
        if let Some(view) = view {
            unsafe {
                let _: () = msg_send![&*view, setCornerRadius: CARD_RADIUS];
                let _: () = msg_send![&*view, setContentView: content];
            }
            return view;
        }
    }
    let view = NSVisualEffectView::initWithFrame(NSVisualEffectView::alloc(mtm), bounds);
    view.setMaterial(NSVisualEffectMaterial::HUDWindow);
    view.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
    view.setState(NSVisualEffectState::Active);
    view.setWantsLayer(true);
    if let Some(layer) = view.layer() {
        layer.setCornerRadius(CARD_RADIUS);
        layer.setMasksToBounds(true);
    }
    view.addSubview(content);
    Retained::into_super(view)
}

#[derive(Clone)]
struct Palette {
    text: Retained<NSColor>,
    secondary: Retained<NSColor>,
    accent: Retained<NSColor>,
    track: CFRetained<CGColor>,
    monogram: CFRetained<CGColor>,
    hairline: CFRetained<CGColor>,
    hover: CFRetained<CGColor>,
}

impl Palette {
    fn new() -> Self {
        let secondary = NSColor::secondaryLabelColor();
        let accent = NSColor::colorWithSRGBRed_green_blue_alpha(
            252.0 / 255.0,
            83.0 / 255.0,
            58.0 / 255.0,
            1.0,
        );
        Self {
            text: NSColor::labelColor(),
            track: NSColor::quaternaryLabelColor().CGColor().into(),
            monogram: accent.colorWithAlphaComponent(0.18).CGColor().into(),
            hairline: secondary.colorWithAlphaComponent(0.35).CGColor().into(),
            hover: NSColor::labelColor()
                .colorWithAlphaComponent(0.08)
                .CGColor()
                .into(),
            secondary,
            accent,
        }
    }
}

struct Painter<'a> {
    view: &'a NSView,
    height: f64,
    palette: Palette,
    body: Retained<NSFont>,
    section: Retained<NSFont>,
    mono: Retained<NSFont>,
    initial: Retained<NSFont>,
    mtm: MainThreadMarker,
}

impl<'a> Painter<'a> {
    fn new(view: &'a NSView, height: f64, mtm: MainThreadMarker) -> Self {
        Self {
            view,
            height,
            palette: Palette::new(),
            body: NSFont::systemFontOfSize_weight(14.0, REGULAR),
            section: NSFont::systemFontOfSize_weight(12.0, SEMIBOLD),
            mono: NSFont::monospacedSystemFontOfSize_weight(11.0, SEMIBOLD),
            initial: NSFont::systemFontOfSize_weight(13.0, SEMIBOLD),
            mtm,
        }
    }

    fn within<'b>(&self, view: &'b NSView, height: f64) -> Painter<'b> {
        Painter {
            view,
            height,
            palette: self.palette.clone(),
            body: self.body.clone(),
            section: self.section.clone(),
            mono: self.mono.clone(),
            initial: self.initial.clone(),
            mtm: self.mtm,
        }
    }

    fn header(&self, mode: Mode) {
        let title = match mode {
            Mode::Pinned => "Pinned apps",
            Mode::Live => "Open apps",
        };
        let width = CARD_W - 2.0 * (PAD + 4.0);
        self.label(title, &self.section, &self.palette.secondary)
            .place(self, PAD + 4.0, PAD, width, HEADER_H);
        let esc = self.label("esc", &self.mono, &self.palette.secondary);
        let (text_w, text_h) = esc.size();
        let (chip_w, chip_h) = (text_w + 10.0, text_h + 2.0);
        let chip_x = CARD_W - PAD - 4.0 - chip_w;
        let chip_top = PAD + (HEADER_H - chip_h) / 2.0;
        self.tile(
            self.frame(chip_x, chip_top, chip_w, chip_h),
            5.0,
            None,
            Some(&self.palette.hairline),
        );
        esc.centered().place(self, chip_x, chip_top, chip_w, chip_h);
    }

    fn row(&self, index: usize, row: &Row) {
        let width = CARD_W - 2.0 * PAD;
        let band = RowView::new(
            self.frame(PAD, row_top(index), width, ROW_H),
            row.key,
            self.palette.hover.clone(),
            self.mtm,
        );
        self.view.addSubview(&band);
        self.within(&band, ROW_H).row_content(row, width);
    }

    fn row_content(&self, row: &Row, width: f64) {
        let top = 0.0;
        let left = ROW_INSET;
        let right = width - ROW_INSET;
        let tile_top = top + (ROW_H - ICON) / 2.0;

        self.tile(
            self.frame(left, tile_top, ICON, ICON),
            6.0,
            Some(&self.palette.track),
            None,
        );
        let badge = row
            .badge
            .map_or_else(|| "·".to_string(), |c| c.to_uppercase().collect());
        self.label(&badge, &self.mono, &self.palette.accent)
            .centered()
            .place(self, left, tile_top, ICON, ICON);

        let icon_x = left + ICON + ROW_INSET;
        match &row.icon {
            Some(icon) => self.icon(icon, icon_x, tile_top),
            None => self.monogram(&row.name, icon_x, tile_top),
        }

        let name_x = icon_x + ICON + ROW_INSET;
        let mut name_right = right;
        if let Some(count) = row.windows.filter(|n| *n > 1) {
            let chip = self.label(&format!("×{count}"), &self.mono, &self.palette.secondary);
            let (text_w, text_h) = chip.size();
            let (chip_w, chip_h) = (text_w + 12.0, text_h + 2.0);
            let chip_x = right - chip_w;
            let chip_top = top + (ROW_H - chip_h) / 2.0;
            self.tile(
                self.frame(chip_x, chip_top, chip_w, chip_h),
                chip_h / 2.0,
                Some(&self.palette.track),
                None,
            );
            chip.centered()
                .place(self, chip_x, chip_top, chip_w, chip_h);
            name_right = chip_x - ROW_INSET;
        }
        self.label(&row.name, &self.body, &self.palette.text).place(
            self,
            name_x,
            top,
            name_right - name_x,
            ROW_H,
        );
    }

    fn note(&self, index: usize, text: &str) {
        let left = PAD + ROW_INSET;
        self.label(text, &self.body, &self.palette.secondary).place(
            self,
            left,
            row_top(index),
            CARD_W - PAD - ROW_INSET - left,
            ROW_H,
        );
    }

    fn icon(&self, icon: &CGImage, x: f64, top: f64) {
        let image = NSImage::initWithCGImage_size(NSImage::alloc(), icon, NSSize::new(ICON, ICON));
        let view = NSImageView::initWithFrame(
            NSImageView::alloc(self.mtm),
            self.frame(x, top, ICON, ICON),
        );
        view.setImage(Some(&image));
        self.view.addSubview(&view);
    }

    fn monogram(&self, name: &str, x: f64, top: f64) {
        self.tile(
            self.frame(x, top, ICON, ICON),
            7.0,
            Some(&self.palette.monogram),
            None,
        );
        let initial = name
            .chars()
            .next()
            .map_or('?', |c| c.to_uppercase().next().unwrap_or(c));
        self.label(&initial.to_string(), &self.initial, &self.palette.accent)
            .centered()
            .place(self, x, top, ICON, ICON);
    }

    fn tile(&self, frame: NSRect, radius: f64, fill: Option<&CGColor>, border: Option<&CGColor>) {
        let view = NSView::initWithFrame(NSView::alloc(self.mtm), frame);
        view.setWantsLayer(true);
        if let Some(layer) = view.layer() {
            layer.setCornerRadius(radius);
            layer.setBackgroundColor(fill);
            if border.is_some() {
                layer.setBorderWidth(1.0);
                layer.setBorderColor(border);
            }
        }
        self.view.addSubview(&view);
    }

    fn label(&self, text: &str, font: &NSFont, color: &NSColor) -> Label {
        let field = NSTextField::labelWithString(&NSString::from_str(text), self.mtm);
        field.setFont(Some(font));
        field.setTextColor(Some(color));
        field.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
        Label(field)
    }

    fn frame(&self, x: f64, top: f64, w: f64, h: f64) -> NSRect {
        NSRect::new(NSPoint::new(x, self.height - top - h), NSSize::new(w, h))
    }
}

struct Label(Retained<NSTextField>);

impl Label {
    fn size(&self) -> (f64, f64) {
        let size = self.0.fittingSize();
        (size.width.ceil(), size.height.ceil())
    }

    fn centered(self) -> Self {
        self.0.setAlignment(NSTextAlignment::Center);
        self
    }

    fn place(self, painter: &Painter<'_>, x: f64, top: f64, w: f64, box_h: f64) {
        let (_, h) = self.size();
        let line_top = top + (box_h - h) / 2.0;
        self.0.setFrame(painter.frame(x, line_top, w, h));
        painter.view.addSubview(&self.0);
    }
}

fn row_top(index: usize) -> f64 {
    PAD + HEADER_H + index as f64 * (ROW_H + ROW_GAP)
}

struct RowIvars {
    key: Option<PhysicalKey>,
    hover: CFRetained<CGColor>,
}

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[ivars = RowIvars]
    struct RowView;

    impl RowView {
        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        #[unsafe(method(hitTest:))]
        fn hit_test(&self, point: NSPoint) -> *mut NSView {
            if self.ivars().key.is_some() && NSPointInRect(point, self.frame()) {
                (self as *const Self).cast_mut().cast()
            } else {
                std::ptr::null_mut()
            }
        }

        #[unsafe(method(mouseEntered:))]
        fn mouse_entered(&self, _event: &NSEvent) {
            self.highlight(true);
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _event: &NSEvent) {
            self.highlight(false);
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, _event: &NSEvent) {}

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) {
            let point = self.convertPoint_fromView(event.locationInWindow(), None);
            if let Some(key) = self.ivars().key
                && NSPointInRect(point, self.bounds())
            {
                crate::synth::post_raw(key.0, true);
                crate::synth::post_raw(key.0, false);
            }
        }
    }
);

impl RowView {
    fn new(
        frame: NSRect,
        key: Option<PhysicalKey>,
        hover: CFRetained<CGColor>,
        mtm: MainThreadMarker,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(RowIvars { key, hover });
        let view: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };
        view.setWantsLayer(true);
        if let Some(layer) = view.layer() {
            layer.setCornerRadius(ROW_RADIUS);
        }
        if key.is_some() {
            let area = unsafe {
                NSTrackingArea::initWithRect_options_owner_userInfo(
                    NSTrackingArea::alloc(),
                    NSRect::ZERO,
                    NSTrackingAreaOptions::MouseEnteredAndExited
                        | NSTrackingAreaOptions::ActiveAlways
                        | NSTrackingAreaOptions::InVisibleRect,
                    Some(&view),
                    None,
                )
            };
            view.addTrackingArea(&area);
        }
        view
    }

    fn highlight(&self, on: bool) {
        if let Some(layer) = self.layer() {
            layer.setBackgroundColor(on.then_some(&*self.ivars().hover));
        }
    }
}

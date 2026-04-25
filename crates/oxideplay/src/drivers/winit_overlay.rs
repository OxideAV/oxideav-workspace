//! On-screen egui overlay for the winit + wgpu video driver.
//!
//! ## What renders
//!
//! After [`crate::drivers::winit_video::VideoRenderer::render`] has
//! drawn the YUV→RGB content into the surface texture, the overlay
//! paints egui meshes on top in a second render pass that uses the
//! same `wgpu::TextureView`. Two semi-transparent dark gradients (top
//! + bottom strips) keep the controls legible on bright frames.
//!
//! Controls (mpv / VLC parity):
//!  - centre play / pause toggle (large)
//!  - bottom seek bar with current / total time
//!  - volume slider + mute toggle
//!  - skip ±10 s
//!  - "i" toggle: a small stats panel (resolution, codec, fps placeholder)
//!
//! ## Auto-hide
//!
//! `last_mouse_move` is updated on every cursor-motion winit event.
//! `should_show()` returns true while the cursor has moved within
//! [`AUTOHIDE_AFTER`], or the player is paused, or we're inside the
//! initial show window. The shown alpha is interpolated against the
//! frame timestamp so fade-in / fade-out happen visually rather than
//! popping.
//!
//! ## Cross-thread safety
//!
//! The overlay is owned by [`crate::drivers::winit_vo::WinitVideoEngine`],
//! which is wrapped in `unsafe impl Send` for `Box<dyn VideoEngine>`.
//! All overlay calls happen on the main thread (winit's event-pump
//! invariant), so no internal locking is needed.

use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{Align2, Color32, FontId, Pos2, Rect, Stroke, StrokeKind, Vec2};

use crate::driver::{OverlayState, PlayerEvent, SeekDir};

/// Cursor-idle threshold past which the overlay fades away during
/// active playback. Matches mpv's default OSD fade-out window.
const AUTOHIDE_AFTER: Duration = Duration::from_secs(3);

/// On a brand-new file (or after the user clicks the window for the
/// first time) we keep the overlay up for this long even without
/// mouse motion, so the user immediately sees what controls exist.
const INITIAL_SHOW: Duration = Duration::from_secs(4);

/// How long the show / hide alpha tween takes. Short, so the UI feels
/// responsive — egui's docs recommend ≤200 ms for chrome animations.
const FADE_DURATION: Duration = Duration::from_millis(180);

/// Opaque overlay UI handle. `paint` is the only entry point worth
/// looking at — the rest is plumbing for egui's stateful immediate
/// mode (font atlas, texture cache, etc.).
pub struct OverlayUi {
    ctx: egui::Context,
    egui_winit: egui_winit::State,
    renderer: egui_wgpu::Renderer,
    /// Last cursor-movement time. Used by [`should_show`] for the
    /// auto-hide policy.
    last_mouse_move: Instant,
    /// Engine state pulled in from the player every tick.
    state: OverlayState,
    /// Buffered events to be drained by [`take_events`] each tick.
    pending_events: Vec<PlayerEvent>,
    /// Toggle for the optional stats panel (resolution, codec).
    show_stats: bool,
    /// True while the cursor is inside the window — winit's
    /// `CursorEntered` / `CursorLeft` flip this.
    cursor_in_window: bool,
    /// Frozen time the overlay first appeared (for `INITIAL_SHOW`).
    init_time: Instant,
    /// Smoothed alpha used by the painter. Interpolated each frame
    /// toward the target driven by `should_show()`.
    visible_alpha: f32,
    last_paint: Instant,
}

impl OverlayUi {
    /// Build the overlay state for an existing wgpu surface. `format`
    /// must match the surface's texture format so egui can render
    /// directly into it. `device` / `queue` are the same handles
    /// `VideoRenderer` already owns.
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        window: &Arc<winit::window::Window>,
    ) -> Self {
        let ctx = egui::Context::default();
        // egui-winit binds context state (modifier keys, IME, the
        // viewport id) to a winit window. `default()` viewport id is
        // fine here — we only own one window.
        let egui_winit = egui_winit::State::new(
            ctx.clone(),
            egui::ViewportId::ROOT,
            window.as_ref(),
            Some(window.scale_factor() as f32),
            None,
            None,
        );
        // `output_color_format` matches the wgpu surface format chosen
        // in VideoRenderer (Bgra8Unorm / Rgba8Unorm). MSAA disabled —
        // the YUV pass that comes before is single-sampled too.
        let renderer =
            egui_wgpu::Renderer::new(device, format, egui_wgpu::RendererOptions::default());
        let now = Instant::now();
        Self {
            ctx,
            egui_winit,
            renderer,
            last_mouse_move: now,
            state: OverlayState::default(),
            pending_events: Vec::new(),
            show_stats: false,
            cursor_in_window: false,
            init_time: now,
            visible_alpha: 1.0,
            last_paint: now,
        }
    }

    /// Forward a winit window event to egui-winit. Returns true if
    /// egui consumed the event (i.e. the user clicked on a control —
    /// the driver should suppress its own keybinding handling for
    /// this event). Also tracks mouse-motion timestamps for the
    /// auto-hide policy.
    pub fn on_window_event(
        &mut self,
        window: &winit::window::Window,
        event: &winit::event::WindowEvent,
    ) -> bool {
        match event {
            winit::event::WindowEvent::CursorMoved { .. } => {
                self.last_mouse_move = Instant::now();
            }
            winit::event::WindowEvent::CursorEntered { .. } => {
                self.cursor_in_window = true;
                self.last_mouse_move = Instant::now();
            }
            winit::event::WindowEvent::CursorLeft { .. } => {
                self.cursor_in_window = false;
            }
            winit::event::WindowEvent::MouseInput { .. }
            | winit::event::WindowEvent::MouseWheel { .. } => {
                // Any mouse interaction counts as activity.
                self.last_mouse_move = Instant::now();
            }
            _ => {}
        }
        let r = self.egui_winit.on_window_event(window, event);
        r.consumed
    }

    /// Update the engine-supplied state snapshot. If the file changed
    /// (codec or video size differs), the overlay resets the
    /// "initial show" timer so the new context is announced for a
    /// few seconds.
    pub fn set_state(&mut self, state: OverlayState) {
        let codec_changed = self.state.codec_name != state.codec_name;
        let size_changed = self.state.video_size != state.video_size;
        if codec_changed || size_changed {
            self.init_time = Instant::now();
        }
        self.state = state;
    }

    /// Drain queued PlayerEvents for the engine to apply.
    pub fn take_events(&mut self) -> Vec<PlayerEvent> {
        std::mem::take(&mut self.pending_events)
    }

    /// Should the overlay be visible right now?
    fn should_show(&self) -> bool {
        if !self.state.playing {
            return true; // always show while paused — VLC-style.
        }
        if self.init_time.elapsed() < INITIAL_SHOW {
            return true;
        }
        if self.cursor_in_window && self.last_mouse_move.elapsed() < AUTOHIDE_AFTER {
            return true;
        }
        false
    }

    /// Render the egui overlay into `target_view`. Called by
    /// `VideoRenderer::render` after the YUV pass has finished,
    /// using the same surface texture view. The overlay never
    /// clears — `LoadOp::Load` keeps the video underneath.
    pub fn paint(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        window: &winit::window::Window,
        target_view: &wgpu::TextureView,
        screen_size: (u32, u32),
    ) {
        // Fade-in/out tween toward the target alpha.
        let now = Instant::now();
        let dt = now.saturating_duration_since(self.last_paint);
        self.last_paint = now;
        let target = if self.should_show() { 1.0 } else { 0.0 };
        let step = dt.as_secs_f32() / FADE_DURATION.as_secs_f32().max(0.001);
        if (self.visible_alpha - target).abs() < step {
            self.visible_alpha = target;
        } else if self.visible_alpha < target {
            self.visible_alpha += step;
        } else {
            self.visible_alpha -= step;
        }
        self.visible_alpha = self.visible_alpha.clamp(0.0, 1.0);

        let raw_input = self.egui_winit.take_egui_input(window);
        let alpha = self.visible_alpha;
        let mut state_snapshot = self.state.clone();
        let mut emitted: Vec<PlayerEvent> = Vec::new();
        let show_stats_in = self.show_stats;
        let mut show_stats_out = show_stats_in;

        let full_output = self.ctx.run_ui(raw_input, |ui| {
            paint_overlay(
                ui.ctx(),
                alpha,
                &mut state_snapshot,
                &mut show_stats_out,
                &mut emitted,
            );
        });
        self.show_stats = show_stats_out;
        self.pending_events.extend(emitted);

        // Hand textures emitted by egui to wgpu (font atlas updates,
        // colour-image uploads). Then build paint jobs.
        for (id, image_delta) in &full_output.textures_delta.set {
            self.renderer
                .update_texture(device, queue, *id, image_delta);
        }
        let paint_jobs = self
            .ctx
            .tessellate(full_output.shapes, full_output.pixels_per_point);

        let screen_descriptor = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [screen_size.0.max(1), screen_size.1.max(1)],
            pixels_per_point: full_output.pixels_per_point,
        };

        self.renderer
            .update_buffers(device, queue, encoder, &paint_jobs, &screen_descriptor);

        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("egui-overlay-pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: target_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                .forget_lifetime();
            self.renderer
                .render(&mut pass, &paint_jobs, &screen_descriptor);
        }

        // Free any textures egui marked as no-longer-needed.
        for id in &full_output.textures_delta.free {
            self.renderer.free_texture(id);
        }
    }
}

// ─────────────────────── widget tree ────────────────────────

fn paint_overlay(
    ctx: &egui::Context,
    alpha: f32,
    state: &mut OverlayState,
    show_stats: &mut bool,
    emit: &mut Vec<PlayerEvent>,
) {
    if alpha <= 0.001 {
        // Fully hidden — request a repaint shortly so we re-evaluate
        // mouse activity and slide back in cleanly.
        ctx.request_repaint_after(Duration::from_millis(500));
        return;
    }
    // Keep the overlay painting smoothly (mpv-style ~30 fps OSD).
    ctx.request_repaint_after(Duration::from_millis(33));

    // `content_rect()` is the egui equivalent of the surface size in
    // points. Used to anchor the bottom controls bar.
    let screen = ctx.content_rect();
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("oxideplay-overlay-bg"),
    ));
    paint_gradients(&painter, screen, alpha);

    // Bottom controls bar — fixed height anchored to bottom edge.
    let bar_h = 96.0;
    let bottom_bar = Rect::from_min_size(
        Pos2::new(screen.left(), screen.bottom() - bar_h),
        Vec2::new(screen.width(), bar_h),
    );
    egui::Area::new(egui::Id::new("oxideplay-overlay-bottom"))
        .fixed_pos(bottom_bar.min)
        .order(egui::Order::Foreground)
        .interactable(true)
        .show(ctx, |ui| {
            ui.set_min_size(bottom_bar.size());
            ui.set_width(bottom_bar.width());
            // Multiply visual style by alpha for fade.
            apply_alpha(ui, alpha);
            paint_bottom_bar(ui, state, show_stats, emit);
        });

    // Optional stats panel (top-left) toggled by the "i" button.
    if *show_stats {
        egui::Window::new("stats")
            .title_bar(false)
            .resizable(false)
            .collapsible(false)
            .anchor(Align2::LEFT_TOP, [16.0, 16.0])
            .show(ctx, |ui| {
                apply_alpha(ui, alpha);
                paint_stats_panel(ui, state);
            });
    }

    // Big centred play indicator while paused — mirrors VLC's "this
    // is paused, click to resume" cue. Only visible when paused.
    if !state.playing {
        let centre = screen.center();
        let r = 56.0;
        painter.circle_filled(centre, r, Color32::from_black_alpha((140.0 * alpha) as u8));
        painter.text(
            centre,
            Align2::CENTER_CENTER,
            "▶",
            FontId::proportional(64.0),
            Color32::from_white_alpha((230.0 * alpha) as u8),
        );
    }
}

fn paint_gradients(painter: &egui::Painter, screen: Rect, alpha: f32) {
    // Bottom gradient strip — solid dark behind the controls so they
    // stay legible on bright frames.
    let strip_h = 140.0;
    let bottom_strip = Rect::from_min_size(
        Pos2::new(screen.left(), screen.bottom() - strip_h),
        Vec2::new(screen.width(), strip_h),
    );
    let dark = Color32::from_black_alpha((180.0 * alpha) as u8);
    painter.rect_filled(bottom_strip, 0.0, dark);

    // Top strip — much shorter, just enough to seat the title.
    let top_strip = Rect::from_min_size(screen.min, Vec2::new(screen.width(), 64.0));
    painter.rect_filled(
        top_strip,
        0.0,
        Color32::from_black_alpha((120.0 * alpha) as u8),
    );

    // Title text top-left.
    painter.text(
        Pos2::new(screen.left() + 16.0, screen.top() + 18.0),
        Align2::LEFT_TOP,
        "oxideplay",
        FontId::proportional(20.0),
        Color32::from_white_alpha((220.0 * alpha) as u8),
    );
}

fn apply_alpha(ui: &mut egui::Ui, alpha: f32) {
    let visuals = ui.visuals_mut();
    visuals.override_text_color = Some(Color32::from_white_alpha((230.0 * alpha) as u8));
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, Color32::TRANSPARENT);
    visuals.widgets.inactive.weak_bg_fill = Color32::from_white_alpha((20.0 * alpha) as u8);
    visuals.widgets.hovered.weak_bg_fill = Color32::from_white_alpha((50.0 * alpha) as u8);
    visuals.widgets.active.weak_bg_fill = Color32::from_white_alpha((90.0 * alpha) as u8);
}

fn paint_bottom_bar(
    ui: &mut egui::Ui,
    state: &mut OverlayState,
    show_stats: &mut bool,
    emit: &mut Vec<PlayerEvent>,
) {
    ui.add_space(12.0);

    // Seek bar — full width, draggable.
    paint_seek_bar(ui, state, emit);
    ui.add_space(6.0);

    ui.horizontal(|ui| {
        ui.add_space(12.0);

        // ◀◀ skip back 10s
        if ui
            .add(egui::Button::new(egui::RichText::new("«10").size(18.0)))
            .on_hover_text("Skip back 10 s")
            .clicked()
        {
            emit.push(PlayerEvent::SeekRelative(
                Duration::from_secs(10),
                SeekDir::Back,
            ));
        }

        // Play / pause toggle.
        let play_label = if state.playing { "❚❚" } else { "▶" };
        if ui
            .add(egui::Button::new(
                egui::RichText::new(play_label).size(22.0),
            ))
            .on_hover_text("Play / Pause")
            .clicked()
        {
            emit.push(PlayerEvent::TogglePause);
            state.playing = !state.playing;
        }

        // Skip forward 10s.
        if ui
            .add(egui::Button::new(egui::RichText::new("10»").size(18.0)))
            .on_hover_text("Skip forward 10 s")
            .clicked()
        {
            emit.push(PlayerEvent::SeekRelative(
                Duration::from_secs(10),
                SeekDir::Forward,
            ));
        }

        ui.add_space(16.0);
        // Time text — fixed width using monospace would be nicer, but
        // proportional is fine at this size. Format MM:SS / MM:SS.
        let pos_str = format_dur(state.position);
        let dur_str = state
            .duration
            .map(format_dur)
            .unwrap_or_else(|| "??:??".into());
        ui.label(
            egui::RichText::new(format!("{pos_str} / {dur_str}"))
                .size(14.0)
                .monospace(),
        );

        // Spacer so the right-hand cluster pins to the edge.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(12.0);

            // Stats toggle.
            if ui
                .add(egui::Button::new(egui::RichText::new("ⓘ").size(18.0)))
                .on_hover_text("Toggle stats")
                .clicked()
            {
                *show_stats = !*show_stats;
            }

            // Volume slider + mute toggle.
            let vol_label = if state.muted {
                "🔇"
            } else if state.volume < 0.01 {
                "🔈"
            } else if state.volume < 0.5 {
                "🔉"
            } else {
                "🔊"
            };
            if ui
                .add(egui::Button::new(egui::RichText::new(vol_label).size(18.0)))
                .on_hover_text("Mute / Unmute")
                .clicked()
            {
                emit.push(PlayerEvent::ToggleMute);
                state.muted = !state.muted;
            }
            let mut vol = state.volume;
            let resp = ui.add(
                egui::Slider::new(&mut vol, 0.0..=1.0)
                    .show_value(false)
                    .clamping(egui::SliderClamping::Always),
            );
            if resp.changed() {
                state.volume = vol;
                emit.push(PlayerEvent::SetVolume(vol));
            }
        });
    });
    ui.add_space(8.0);
}

fn paint_seek_bar(ui: &mut egui::Ui, state: &mut OverlayState, emit: &mut Vec<PlayerEvent>) {
    let dur = state.duration.unwrap_or(Duration::ZERO);
    let dur_secs = dur.as_secs_f64();
    let pos_secs = state.position.as_secs_f64();

    let avail = ui.available_width() - 24.0;
    let height = 18.0;
    let (rect, resp) =
        ui.allocate_exact_size(Vec2::new(avail, height), egui::Sense::click_and_drag());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter_at(rect);

    // Track background.
    let track_h = 4.0;
    let track = Rect::from_min_size(
        Pos2::new(rect.left(), rect.center().y - track_h * 0.5),
        Vec2::new(rect.width(), track_h),
    );
    painter.rect_filled(track, 2.0, Color32::from_white_alpha(60));

    let seekable = state.seekable && dur_secs > 0.0;
    let frac = if dur_secs > 0.0 {
        (pos_secs / dur_secs).clamp(0.0, 1.0) as f32
    } else {
        0.0
    };

    // Filled portion (current position).
    let fill_color = if seekable {
        Color32::from_rgba_unmultiplied(0xff, 0x6a, 0x00, 220)
    } else {
        Color32::from_white_alpha(120)
    };
    let fill = Rect::from_min_size(track.min, Vec2::new(track.width() * frac, track.height()));
    painter.rect_filled(fill, 2.0, fill_color);

    // Thumb.
    let thumb_x = rect.left() + rect.width() * frac;
    let thumb_y = rect.center().y;
    painter.circle_filled(Pos2::new(thumb_x, thumb_y), 7.0, fill_color);
    painter.circle_stroke(
        Pos2::new(thumb_x, thumb_y),
        7.0,
        Stroke::new(1.0, Color32::from_white_alpha(180)),
    );

    if seekable && (resp.clicked() || resp.dragged()) {
        if let Some(pos) = resp.interact_pointer_pos() {
            let f = ((pos.x - rect.left()) / rect.width().max(1.0)).clamp(0.0, 1.0);
            let target = Duration::from_secs_f64(dur_secs * f as f64);
            state.position = target;
            emit.push(PlayerEvent::SeekAbsolute(target));
        }
    }

    // Hover preview — show the timestamp under the cursor.
    if seekable {
        if let Some(hp) = resp.hover_pos() {
            let f = ((hp.x - rect.left()) / rect.width().max(1.0)).clamp(0.0, 1.0);
            let t = Duration::from_secs_f64(dur_secs * f as f64);
            let label_pos = Pos2::new(hp.x, rect.top() - 6.0);
            let txt = format_dur(t);
            let tip_rect = Rect::from_center_size(
                Pos2::new(label_pos.x, label_pos.y - 8.0),
                Vec2::new(56.0, 18.0),
            );
            painter.rect(
                tip_rect,
                3.0,
                Color32::from_black_alpha(200),
                Stroke::NONE,
                StrokeKind::Outside,
            );
            painter.text(
                tip_rect.center(),
                Align2::CENTER_CENTER,
                txt,
                FontId::monospace(12.0),
                Color32::WHITE,
            );
        }
    }
}

fn paint_stats_panel(ui: &mut egui::Ui, state: &OverlayState) {
    ui.set_min_width(220.0);
    ui.label(egui::RichText::new("Stream").strong());
    ui.separator();
    if let Some((w, h)) = state.video_size {
        ui.label(format!("Resolution: {w}×{h}"));
    } else {
        ui.label("Resolution: ?");
    }
    if let Some(c) = state.codec_name.as_ref() {
        ui.label(format!("Codec: {c}"));
    }
    if let Some(d) = state.duration {
        ui.label(format!("Duration: {}", format_dur(d)));
    }
    ui.label(format!("Volume: {:.0}%", state.volume * 100.0));
    if state.muted {
        ui.colored_label(Color32::YELLOW, "Muted");
    }
    if !state.seekable {
        ui.colored_label(Color32::LIGHT_RED, "Source not seekable");
    }
}

/// Format a `Duration` as `MM:SS` or `HH:MM:SS` if ≥ 1 hour.
fn format_dur(d: Duration) -> String {
    let s = d.as_secs();
    let h = s / 3600;
    let m = (s % 3600) / 60;
    let s = s % 60;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_dur_under_hour() {
        assert_eq!(format_dur(Duration::from_secs(0)), "00:00");
        assert_eq!(format_dur(Duration::from_secs(65)), "01:05");
        assert_eq!(format_dur(Duration::from_secs(599)), "09:59");
    }

    #[test]
    fn format_dur_over_hour() {
        assert_eq!(format_dur(Duration::from_secs(3661)), "1:01:01");
        assert_eq!(format_dur(Duration::from_secs(7200)), "2:00:00");
    }
}

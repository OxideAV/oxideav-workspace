//! egui HUD for the viewer: a status panel (backend, adapter,
//! triangles, fps, mode, camera) and a key-help panel, painted over
//! the blitted frame. Display-only — all input goes to the viewer.

use std::sync::Arc;

use egui::{Align2, Color32, RichText};

use super::{Hud, HELP};

pub struct HudUi {
    ctx: egui::Context,
    egui_winit: egui_winit::State,
    renderer: egui_wgpu::Renderer,
}

impl HudUi {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        window: &Arc<winit::window::Window>,
    ) -> Self {
        let ctx = egui::Context::default();
        let egui_winit = egui_winit::State::new(
            ctx.clone(),
            egui::ViewportId::ROOT,
            window.as_ref(),
            Some(window.scale_factor() as f32),
            None,
            None,
        );
        let renderer =
            egui_wgpu::Renderer::new(device, format, egui_wgpu::RendererOptions::default());
        Self {
            ctx,
            egui_winit,
            renderer,
        }
    }

    /// Keep egui's view of the window (size, scale factor) current.
    pub fn on_window_event(
        &mut self,
        window: &winit::window::Window,
        event: &winit::event::WindowEvent,
    ) {
        let _ = self.egui_winit.on_window_event(window, event);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn paint(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        window: &winit::window::Window,
        target: &wgpu::TextureView,
        screen: (u32, u32),
        hud: &Hud,
    ) {
        let raw = self.egui_winit.take_egui_input(window);
        let full = self.ctx.run_ui(raw, |ui| draw(ui.ctx(), hud));
        for (id, delta) in &full.textures_delta.set {
            self.renderer.update_texture(device, queue, *id, delta);
        }
        let jobs = self.ctx.tessellate(full.shapes, full.pixels_per_point);
        let desc = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [screen.0.max(1), screen.1.max(1)],
            pixels_per_point: full.pixels_per_point,
        };
        self.renderer
            .update_buffers(device, queue, encoder, &jobs, &desc);
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("viewer-hud"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: target,
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
            self.renderer.render(&mut pass, &jobs, &desc);
        }
        for id in &full.textures_delta.free {
            self.renderer.free_texture(id);
        }
    }
}

fn panel(ctx: &egui::Context, id: &str, anchor: Align2, offset: [f32; 2]) -> egui::Window<'static> {
    egui::Window::new(id.to_string())
        .title_bar(false)
        .resizable(false)
        .collapsible(false)
        .interactable(false)
        .anchor(anchor, offset)
        .frame(
            egui::Frame::window(&ctx.global_style())
                .fill(Color32::from_black_alpha(170))
                .stroke(egui::Stroke::NONE),
        )
}

fn draw(ctx: &egui::Context, hud: &Hud) {
    if !hud.visible {
        panel(
            ctx,
            "viewer-hint",
            Align2::LEFT_BOTTOM,
            [10.0_f32, -10.0_f32],
        )
        .show(ctx, |ui| {
            ui.label(RichText::new("H: show HUD").color(Color32::from_white_alpha(160)));
        });
        return;
    }
    panel(ctx, "viewer-status", Align2::LEFT_TOP, [10.0_f32, 10.0_f32]).show(ctx, |ui| {
        egui::Grid::new("viewer-status-grid")
            .num_columns(2)
            .spacing([12.0_f32, 2.0_f32])
            .show(ui, |ui| {
                for (k, v) in &hud.rows {
                    ui.label(RichText::new(k).color(Color32::from_gray(170)));
                    let text = RichText::new(v).color(Color32::WHITE);
                    if k == "error" {
                        ui.label(text.color(Color32::LIGHT_RED));
                    } else {
                        ui.label(text);
                    }
                    ui.end_row();
                }
            });
    });
    panel(ctx, "viewer-help", Align2::RIGHT_TOP, [-10.0_f32, 10.0_f32]).show(ctx, |ui| {
        egui::Grid::new("viewer-help-grid")
            .num_columns(2)
            .spacing([12.0_f32, 1.0_f32])
            .show(ui, |ui| {
                for (k, v) in HELP {
                    ui.label(RichText::new(*k).monospace().color(Color32::from_gray(220)));
                    ui.label(RichText::new(*v).color(Color32::from_gray(170)));
                    ui.end_row();
                }
            });
    });
}

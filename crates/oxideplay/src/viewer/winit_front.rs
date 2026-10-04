//! winit + wgpu viewer window.
//!
//! Owns its own window, surface and wgpu device (separate from the
//! media player's `VideoRenderer`, which is YUV-specific). The current
//! frame is a texture holding sRGB-encoded RGBA8 — either uploaded
//! from a software [`RgbaImage`] or drawn on the GPU by
//! `oxideav-render-vulkan` sharing this device — and a fullscreen
//! triangle blits it onto a UNORM view of the surface (no double sRGB
//! encode even when the surface format is `*Srgb`). The egui HUD, when
//! compiled in, paints on top.

use std::sync::Arc;
use std::time::Duration;

use oxideav_core::{Error, Result};
#[cfg(feature = "viewer-gpu")]
use oxideav_render::RenderOptions;
use oxideav_render::RgbaImage;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key as WKey, NamedKey};
use winit::platform::pump_events::{EventLoopExtPumpEvents, PumpStatus};
use winit::window::{Fullscreen, Window, WindowAttributes, WindowId};

use super::state::{InputEvent, Key, MouseButton};
use super::{Frontend, Hud};

/// Initial window size.
const INITIAL_SIZE: (u32, u32) = (1024, 768);

pub struct WinitFrontend {
    event_loop: EventLoop<()>,
    app: App,
}

struct App {
    title: String,
    window: Option<Arc<Window>>,
    gfx: Option<Gfx>,
    init_error: Option<String>,
    events: Vec<InputEvent>,
    cursor: (f64, f64),
}

/// Which texture the blit samples.
enum Current {
    None,
    Soft,
    #[cfg(feature = "viewer-gpu")]
    Gpu,
}

struct Gfx {
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    surface_cfg: wgpu::SurfaceConfiguration,
    /// UNORM format the blit / HUD render into (a view format of the
    /// surface when the surface itself is sRGB).
    target_format: wgpu::TextureFormat,
    max_dim: u32,
    blit: super::blit::Blit,
    soft_tex: Option<(wgpu::Texture, u32, u32)>,
    bind_group: Option<wgpu::BindGroup>,
    current: Current,
    adapter_summary: String,
    last_title: String,
    #[cfg(feature = "viewer-gpu")]
    gpu: oxideav_render_vulkan::GpuRenderer,
    #[cfg(feature = "viewer-gpu")]
    gpu_scene: Option<oxideav_render_vulkan::GpuScene>,
    #[cfg(feature = "egui")]
    hud: super::hud_egui::HudUi,
    window: Arc<Window>,
}

impl WinitFrontend {
    pub fn new(title: &str) -> Result<Self> {
        let mut event_loop =
            EventLoop::new().map_err(|e| Error::other(format!("winit: EventLoop::new: {e}")))?;
        let mut app = App {
            title: title.to_string(),
            window: None,
            gfx: None,
            init_error: None,
            events: Vec::new(),
            cursor: (0.0, 0.0),
        };
        // Pump until `resumed()` has built the window + device.
        for _ in 0..50 {
            let _ = event_loop.pump_app_events(Some(Duration::from_millis(10)), &mut app);
            if app.gfx.is_some() || app.init_error.is_some() {
                break;
            }
        }
        if let Some(e) = app.init_error.take() {
            return Err(Error::other(e));
        }
        if app.gfx.is_none() {
            return Err(Error::other("winit: window never became ready"));
        }
        Ok(Self { event_loop, app })
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = WindowAttributes::default()
            .with_title(self.title.clone())
            .with_inner_size(winit::dpi::LogicalSize::new(INITIAL_SIZE.0, INITIAL_SIZE.1));
        let window = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                self.init_error = Some(format!("winit: create_window: {e}"));
                return;
            }
        };
        match pollster::block_on(Gfx::new(window.clone())) {
            Ok(g) => self.gfx = Some(g),
            Err(e) => self.init_error = Some(e.to_string()),
        }
        self.window = Some(window);
    }

    fn window_event(&mut self, _el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        #[cfg(feature = "egui")]
        if let Some(g) = self.gfx.as_mut() {
            g.hud.on_window_event(&g.window, &event);
        }
        match event {
            WindowEvent::CloseRequested => self.events.push(InputEvent::CloseRequested),
            WindowEvent::Resized(size) => {
                if let Some(g) = self.gfx.as_mut() {
                    g.resize(size.width, size.height);
                }
                self.events
                    .push(InputEvent::Resized(size.width, size.height));
            }
            WindowEvent::ModifiersChanged(m) => {
                self.events.push(InputEvent::Shift(m.state().shift_key()));
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = (position.x, position.y);
                self.events.push(InputEvent::MouseMove {
                    x: position.x,
                    y: position.y,
                });
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let button = match button {
                    winit::event::MouseButton::Left => MouseButton::Left,
                    winit::event::MouseButton::Right => MouseButton::Right,
                    winit::event::MouseButton::Middle => MouseButton::Middle,
                    _ => return,
                };
                self.events.push(match state {
                    ElementState::Pressed => InputEvent::MouseDown {
                        button,
                        x: self.cursor.0,
                        y: self.cursor.1,
                    },
                    ElementState::Released => InputEvent::MouseUp { button },
                });
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let notches = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => (p.y / 40.0) as f32,
                };
                self.events.push(InputEvent::Wheel(notches));
            }
            WindowEvent::KeyboardInput { event: key, .. } if key.state == ElementState::Pressed => {
                let mapped = match &key.logical_key {
                    WKey::Named(NamedKey::Escape) => Some(Key::Escape),
                    WKey::Named(NamedKey::Home) => Some(Key::Home),
                    WKey::Named(NamedKey::F1) => Some(Key::F1),
                    WKey::Named(NamedKey::Space) => Some(Key::Char(' ')),
                    WKey::Character(s) => {
                        s.chars().next().map(|c| Key::Char(c.to_ascii_lowercase()))
                    }
                    _ => None,
                };
                // Auto-repeat only for the zoom keys; toggles fire once.
                if let Some(k) = mapped {
                    if !key.repeat || matches!(k, Key::Char('+' | '=' | '-' | '_')) {
                        self.events.push(InputEvent::Key(k));
                    }
                }
            }
            _ => {}
        }
    }
}

impl Gfx {
    async fn new(window: Arc<Window>) -> Result<Self> {
        let size = window.inner_size();
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            flags: wgpu::InstanceFlags::default(),
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
            backend_options: wgpu::BackendOptions::default(),
            display: None,
        });
        let surface = instance
            .create_surface(window.clone())
            .map_err(|e| Error::other(format!("wgpu: create_surface: {e}")))?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .map_err(|e| Error::other(format!("wgpu: no suitable adapter: {e}")))?;
        let info = adapter.get_info();
        let limits = adapter.limits();
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("oxideplay-viewer"),
                required_features: wgpu::Features::empty(),
                required_limits: limits.clone(),
                memory_hints: wgpu::MemoryHints::default(),
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                trace: wgpu::Trace::Off,
            })
            .await
            .map_err(|e| Error::other(format!("wgpu: request_device: {e}")))?;
        let max_dim = limits.max_texture_dimension_2d;

        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| {
                matches!(
                    f,
                    wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Rgba8Unorm
                )
            })
            .or_else(|| caps.formats.first().copied())
            .ok_or_else(|| Error::other("wgpu: surface reports no formats"))?;
        let target_format = format.remove_srgb_suffix();
        let view_formats = if target_format != format {
            vec![target_format]
        } else {
            vec![]
        };
        let surface_cfg = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.clamp(1, max_dim),
            height: size.height.clamp(1, max_dim),
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats,
        };
        surface.configure(&device, &surface_cfg);

        let blit = super::blit::Blit::new(&device, target_format);

        let adapter_summary = format!(
            "{} ({:?}, {:?})",
            if info.name.is_empty() {
                "<unnamed adapter>"
            } else {
                info.name.as_str()
            },
            info.device_type,
            info.backend
        )
        .to_lowercase();

        #[cfg(feature = "egui")]
        let hud = super::hud_egui::HudUi::new(&device, target_format, &window);

        #[cfg(feature = "viewer-gpu")]
        let gpu = oxideav_render_vulkan::GpuRenderer::from_device(
            device.clone(),
            queue.clone(),
            info.clone(),
        );

        Ok(Self {
            device,
            queue,
            surface,
            surface_cfg,
            target_format,
            max_dim,
            blit,
            soft_tex: None,
            bind_group: None,
            current: Current::None,
            adapter_summary,
            last_title: String::new(),
            #[cfg(feature = "viewer-gpu")]
            gpu,
            #[cfg(feature = "viewer-gpu")]
            gpu_scene: None,
            #[cfg(feature = "egui")]
            hud,
            window,
        })
    }

    fn resize(&mut self, w: u32, h: u32) {
        self.surface_cfg.width = w.clamp(1, self.max_dim);
        self.surface_cfg.height = h.clamp(1, self.max_dim);
        self.surface.configure(&self.device, &self.surface_cfg);
    }

    fn bind(&mut self, view: &wgpu::TextureView) {
        self.bind_group = Some(self.blit.bind_group(&self.device, view));
    }

    fn set_image(&mut self, img: &RgbaImage) -> Result<()> {
        let (w, h) = (img.width, img.height);
        if w == 0 || h == 0 || w > self.max_dim || h > self.max_dim {
            return Ok(());
        }
        let stride = img.stride.max(w as usize * 4);
        if img.pixels.len() < stride * (h as usize - 1) + w as usize * 4 {
            return Err(Error::invalid("viewer: short RGBA image"));
        }
        let fresh = !matches!(&self.soft_tex, Some((_, tw, th)) if (*tw, *th) == (w, h));
        if fresh {
            let tex = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("viewer-soft-frame"),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                // UNORM holding sRGB-encoded bytes, like the GPU path.
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            self.soft_tex = Some((tex, w, h));
        }
        let Some((tex, _, _)) = self.soft_tex.as_ref() else {
            return Ok(());
        };
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &img.pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride as u32),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        if fresh || !matches!(self.current, Current::Soft) {
            let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
            self.bind(&view);
        }
        self.current = Current::Soft;
        Ok(())
    }

    #[cfg(feature = "viewer-gpu")]
    fn gpu_draw(&mut self, opts: &RenderOptions) -> Result<()> {
        let Some(scene) = self.gpu_scene.as_mut() else {
            return Err(Error::unsupported("viewer: no scene uploaded to the GPU"));
        };
        // The renderer supersamples (`opts.aa`) and resolves on the GPU.
        let mut opts = opts.clone();
        opts.width = opts.width.min(self.max_dim);
        opts.height = opts.height.min(self.max_dim);
        let tex = self
            .gpu
            .draw(scene, &opts)
            .map_err(|e| Error::other(format!("gpu draw: {e}")))?;
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        self.bind(&view);
        self.current = Current::Gpu;
        Ok(())
    }

    fn present(&mut self, hud: &Hud) -> Result<()> {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.surface_cfg);
                match self.surface.get_current_texture() {
                    wgpu::CurrentSurfaceTexture::Success(t)
                    | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
                    _ => return Ok(()),
                }
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err(Error::other("wgpu: surface texture validation error"));
            }
        };
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(self.target_format),
            ..Default::default()
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("viewer-present"),
            });
        let source = match self.current {
            Current::None => None,
            _ => self.bind_group.as_ref(),
        };
        self.blit.encode(&mut encoder, &view, source);
        #[cfg(feature = "egui")]
        {
            let size = (self.surface_cfg.width, self.surface_cfg.height);
            self.hud.paint(
                &self.device,
                &self.queue,
                &mut encoder,
                &self.window,
                &view,
                size,
                hud,
            );
        }
        if hud.summary != self.last_title {
            self.window.set_title(&hud.summary);
            self.last_title = hud.summary.clone();
        }
        self.queue.submit(Some(encoder.finish()));
        frame.present();
        Ok(())
    }
}

impl Frontend for WinitFrontend {
    fn pump(&mut self, timeout: Duration) -> Vec<InputEvent> {
        if let PumpStatus::Exit(_) = self
            .event_loop
            .pump_app_events(Some(timeout), &mut self.app)
        {
            self.app.events.push(InputEvent::CloseRequested);
        }
        std::mem::take(&mut self.app.events)
    }

    fn size(&self) -> (u32, u32) {
        self.app
            .gfx
            .as_ref()
            .map(|g| (g.surface_cfg.width, g.surface_cfg.height))
            .unwrap_or(INITIAL_SIZE)
    }

    fn set_image(&mut self, img: &RgbaImage) -> Result<()> {
        match self.app.gfx.as_mut() {
            Some(g) => g.set_image(img),
            None => Ok(()),
        }
    }

    fn present(&mut self, hud: &Hud) -> Result<()> {
        match self.app.gfx.as_mut() {
            Some(g) => g.present(hud),
            None => Ok(()),
        }
    }

    fn toggle_fullscreen(&mut self) {
        if let Some(w) = self.app.window.as_ref() {
            let next = if w.fullscreen().is_some() {
                None
            } else {
                Some(Fullscreen::Borderless(None))
            };
            w.set_fullscreen(next);
        }
    }

    fn describe(&self) -> String {
        match self.app.gfx.as_ref() {
            Some(g) => format!(
                "winit+wgpu  {}  surface {:?}{}",
                g.adapter_summary,
                g.surface_cfg.format,
                if cfg!(feature = "viewer-gpu") {
                    "  (gpu render: oxideav-render-vulkan)"
                } else {
                    "  (software render only)"
                }
            ),
            None => "winit+wgpu (not ready)".into(),
        }
    }

    #[cfg(feature = "viewer-gpu")]
    fn set_texture_resolver(&mut self, resolver: Arc<dyn oxideav_render::TextureResolver>) {
        if let Some(g) = self.app.gfx.as_mut() {
            g.gpu.set_texture_resolver(resolver);
        }
    }

    #[cfg(feature = "viewer-gpu")]
    fn gpu_adapter(&self) -> Option<String> {
        self.app.gfx.as_ref().map(|g| g.adapter_summary.clone())
    }

    #[cfg(feature = "viewer-gpu")]
    fn gpu_upload(
        &mut self,
        scene: &oxideav_mesh3d::Scene3D,
        opts: &RenderOptions,
    ) -> Option<usize> {
        let g = self.app.gfx.as_mut()?;
        let gs = g.gpu.upload(scene, opts);
        let n = gs.triangle_count();
        g.gpu_scene = Some(gs);
        Some(n)
    }

    #[cfg(feature = "viewer-gpu")]
    fn gpu_draw(&mut self, opts: &RenderOptions) -> Result<()> {
        match self.app.gfx.as_mut() {
            Some(g) => g.gpu_draw(opts),
            None => Err(Error::other("viewer: window not ready")),
        }
    }
}

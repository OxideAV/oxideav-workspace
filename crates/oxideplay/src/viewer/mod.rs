//! Interactive 3D model viewer.
//!
//! `oxideplay model.glb` (any extension the `oxideav-mesh3d` registry
//! knows) lands here instead of the media pipeline. The model is
//! decoded once into a [`Scene3D`] and shown in a window with an orbit
//! camera; see [`state`] for the input surface.
//!
//! Two render paths:
//!
//! * **GPU** (`viewer-gpu` feature, winit window only):
//!   `oxideav-render-vulkan` shares the window's wgpu device, keeps the
//!   scene resident and draws straight into an offscreen texture that
//!   is blitted onto the surface — no readback.
//! * **Software** (any window): `oxideav-render`'s CPU backends
//!   (`scanline`, `raycast`, …) on a worker thread, with reduced
//!   resolution while dragging and progressive refinement when idle
//!   (see [`soft`]). Frames are pushed to the window as RGBA images.
//!
//! `B` cycles through every available backend.

// Without a window output compiled in, the viewer can only report that
// it has nowhere to draw; its input / HUD plumbing goes unused.
#![cfg_attr(not(any(feature = "winit", feature = "sdl2")), allow(dead_code))]

pub mod state;

mod soft;

#[cfg(feature = "winit")]
mod blit;
#[cfg(feature = "viewer-gpu")]
mod gpu_pt;
#[cfg(all(feature = "winit", feature = "egui"))]
mod hud_egui;
#[cfg(feature = "sdl2")]
mod sdl_front;
#[cfg(feature = "winit")]
mod winit_front;

use std::sync::Arc;
use std::time::{Duration, Instant};

use oxideav_core::{Error, Result};
use oxideav_mesh3d::{Indices, Mesh3DRegistry, Scene3D, Topology};
use oxideav_render::{RegistryTextureResolver, RenderOptions, RgbaImage, TextureResolver};

use self::soft::{
    adapt_preview_div, plan_passes, preemptible, software_backends, Pass, Progress, SoftWorker,
    PATHTRACE,
};
use self::state::{shading_name, Command, InputEvent, ViewerState, AA_LEVEL};

/// Help lines shown in the HUD / printed at startup: `(keys, action)`.
pub const HELP: &[(&str, &str)] = &[
    ("left drag", "orbit"),
    ("shift+drag / L", "move light"),
    ("right/mid drag", "pan"),
    ("wheel, + / -", "zoom"),
    ("R / Home", "reset view"),
    ("1-7 / M", "shading mode"),
    ("P", "perspective / ortho"),
    ("A", "anti-aliasing"),
    ("[ / ] / 0", "exposure -/+/reset"),
    ("T", "tone map"),
    ("G", "path-trace GI on/off"),
    (", / .", "ambient -/+"),
    ("B", "cycle backend"),
    ("space", "play anim / turntable"),
    ("O", "turntable"),
    ("H / F1", "toggle HUD"),
    ("F", "fullscreen"),
    ("Q / Esc", "quit"),
];

/// Snapshot handed to the frontend for its overlay / title.
#[derive(Clone, Debug, Default)]
pub struct Hud {
    pub visible: bool,
    /// Short one-liner (window title on frontends without an overlay).
    pub summary: String,
    /// `(label, value)` status rows.
    pub rows: Vec<(String, String)>,
}

/// A window the viewer can draw into. Implemented by the winit and
/// SDL2 frontends; both accept software frames, only winit (with
/// `viewer-gpu`) offers the GPU path.
pub trait Frontend {
    /// Wait up to `timeout` for input and return what arrived.
    fn pump(&mut self, timeout: Duration) -> Vec<InputEvent>;
    /// Drawable size in physical pixels.
    fn size(&self) -> (u32, u32);
    /// Make `img` the current frame (shown on the next [`Self::present`]).
    fn set_image(&mut self, img: &RgbaImage) -> Result<()>;
    /// Show the current frame plus the HUD.
    fn present(&mut self, hud: &Hud) -> Result<()>;
    fn toggle_fullscreen(&mut self);
    /// One-line description for the startup banner.
    fn describe(&self) -> String;
    /// Install the image decoder the GPU path uses for textures.
    fn set_texture_resolver(&mut self, _resolver: Arc<dyn TextureResolver>) {}
    /// GPU adapter summary when the GPU path is available.
    fn gpu_adapter(&self) -> Option<String> {
        None
    }
    /// Upload the scene for the GPU path, posed per `opts.time` /
    /// `opts.animation`; returns its triangle count.
    fn gpu_upload(&mut self, _scene: &Scene3D, _opts: &RenderOptions) -> Option<usize> {
        None
    }
    /// Draw the uploaded scene with `opts` (supersampled by `opts.aa`)
    /// and make it the current frame.
    fn gpu_draw(&mut self, _opts: &RenderOptions) -> Result<()> {
        Err(Error::unsupported("viewer: GPU path not available"))
    }
    /// Start (on first call) / poll the lazy creation of the GPU path
    /// tracer. `None`: no GPU path tracing on this frontend.
    fn gpu_pt_poll(&mut self) -> Option<GpuPtStatus> {
        None
    }
    /// One progressive GPU path-tracing frame (sync, refine up to
    /// `spp` samples, resolve) made the current frame.
    fn gpu_pt_frame(
        &mut self,
        _scene: &Scene3D,
        _opts: &RenderOptions,
        _spp: u32,
    ) -> Result<Progress> {
        Err(Error::unsupported("viewer: GPU path tracer not available"))
    }
}

/// State of the lazily created GPU path tracer. Only frontends with
/// the `viewer-gpu` path ever report `Ready`.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(not(feature = "viewer-gpu"), allow(dead_code))]
pub enum GpuPtStatus {
    /// Kernel compiling (first use in the process takes ~1.5 s).
    Compiling,
    Ready,
    Failed(String),
}

/// Samples per frame the GPU path tracer starts at (adapted per frame
/// by `gpu_pt::adapt_spp`).
const GPU_PT_START_SPP: u32 = 2;

/// Next GPU path-trace samples-per-frame from the last frame interval.
fn adapt_gpu_spp(spp: u32, interval: Duration) -> u32 {
    #[cfg(feature = "viewer-gpu")]
    {
        gpu_pt::adapt_spp(spp, interval)
    }
    #[cfg(not(feature = "viewer-gpu"))]
    {
        let _ = interval;
        spp
    }
}

/// Lowercased extension of a local path / `file://` URL, or `None`
/// for remote URLs and extension-less paths.
fn local_extension(input: &str) -> Option<String> {
    if input.contains("://") && !input.starts_with("file://") {
        return None;
    }
    let path = input.strip_prefix("file://").unwrap_or(input);
    let ext = std::path::Path::new(path).extension()?.to_str()?;
    Some(ext.to_ascii_lowercase())
}

fn mesh3d_registry() -> Mesh3DRegistry {
    let mut reg = Mesh3DRegistry::new();
    oxideav_meta::populate_mesh3d_registry(&mut reg);
    reg
}

/// True when `input` is a local file whose extension a registered 3D
/// decoder claims.
pub fn is_model_input(input: &str) -> bool {
    local_extension(input)
        .is_some_and(|ext| mesh3d_registry().decoder_for_extension(&ext).is_some())
}

/// Decode a model file into a [`Scene3D`].
pub fn load_scene(input: &str) -> Result<Scene3D> {
    let ext = local_extension(input)
        .ok_or_else(|| Error::invalid(format!("viewer: '{input}' has no file extension")))?;
    let mut decoder = mesh3d_registry()
        .decoder_for_extension(&ext)
        .ok_or_else(|| Error::unsupported(format!("viewer: no 3D decoder for '.{ext}'")))?;
    let path = input.strip_prefix("file://").unwrap_or(input);
    let bytes =
        std::fs::read(path).map_err(|e| Error::invalid(format!("viewer: read {path}: {e}")))?;
    decoder
        .decode(&bytes)
        .map_err(|e| Error::invalid(format!("viewer: decode {path}: {e}")))
}

/// Texture decoder over the framework's codec registry, resolving
/// relative texture URIs against the model's directory.
pub fn texture_resolver(input: &str) -> Arc<dyn TextureResolver> {
    let mut ctx = oxideav_core::RuntimeContext::new();
    oxideav_meta::register_all(&mut ctx);
    let mut r = RegistryTextureResolver::new(Arc::new(ctx));
    let path = std::path::Path::new(input.strip_prefix("file://").unwrap_or(input));
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        r = r.with_base_dir(dir);
    }
    Arc::new(r)
}

/// Largest axis of the scene's world bounding box (1.0 when empty) —
/// the same extent the renderer frames the orbit on.
pub fn scene_extent(scene: &Scene3D) -> f32 {
    scene
        .bounding_box()
        .map(|b| (0..3).map(|i| b.max[i] - b.min[i]).fold(0.0_f32, f32::max))
        .filter(|e| e.is_finite() && *e > 0.0)
        .unwrap_or(1.0)
}

/// Triangles drawn per frame: every mesh instance (node) counted.
pub fn triangle_count(scene: &Scene3D) -> usize {
    let per_mesh: Vec<usize> = scene
        .meshes
        .iter()
        .map(|m| {
            m.primitives
                .iter()
                .map(|p| {
                    let n = match &p.indices {
                        Some(Indices::U16(v)) => v.len(),
                        Some(Indices::U32(v)) => v.len(),
                        None => p.positions.len(),
                    };
                    match p.topology {
                        Topology::Triangles => n / 3,
                        Topology::TriangleStrip | Topology::TriangleFan => n.saturating_sub(2),
                        _ => 0,
                    }
                })
                .sum()
        })
        .collect();
    let instanced: usize = scene
        .nodes
        .iter()
        .filter_map(|n| n.mesh.and_then(|m| per_mesh.get(m.0 as usize)))
        .sum();
    if instanced > 0 {
        instanced
    } else {
        per_mesh.iter().sum()
    }
}

/// `Some(duration)` when the scene has animations: the longest
/// keyframe time across the first animation's channels (`None` inside
/// when it can't be determined).
pub fn animation_duration(scene: &Scene3D) -> Option<Option<f32>> {
    let anim = scene.animations.first()?;
    let end = anim
        .channels
        .iter()
        .filter_map(|c| c.sampler.keyframes.last().copied())
        .filter(|t| t.is_finite())
        .fold(0.0_f32, f32::max);
    Some((end > 0.0).then_some(end))
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Backend {
    /// GPU rasteriser (`oxideav-render-vulkan`).
    Gpu,
    /// GPU progressive path tracer on the same device.
    GpuPt,
    /// A CPU backend from the render registry, by name.
    Soft(String),
}

/// The `B` cycle: GPU raster → GPU path trace (when a GPU is up) →
/// the CPU backends (scanline → raycast → path trace).
fn backend_list(gpu: bool, soft: Vec<String>) -> Vec<Backend> {
    let mut out = Vec::new();
    if gpu {
        out.push(Backend::Gpu);
        out.push(Backend::GpuPt);
    }
    out.extend(soft.into_iter().map(Backend::Soft));
    out
}

impl Backend {
    fn label(&self) -> String {
        match self {
            Backend::Gpu => "gpu".into(),
            Backend::GpuPt => "gpu-pathtrace".into(),
            Backend::Soft(n) => n.clone(),
        }
    }
}

/// Rolling frames-per-second counter.
struct Fps {
    window_start: Instant,
    frames: u32,
    value: f32,
}

impl Fps {
    fn new() -> Self {
        Self {
            window_start: Instant::now(),
            frames: 0,
            value: 0.0,
        }
    }
    fn frame(&mut self) {
        self.frames += 1;
        let el = self.window_start.elapsed();
        if el >= Duration::from_millis(500) {
            self.value = self.frames as f32 / el.as_secs_f32();
            self.frames = 0;
            self.window_start = Instant::now();
        }
    }
}

/// Run the viewer on `input` with the video output named by `vo`
/// (`auto`, `winit`, `sdl2`).
pub fn run(input: &str, vo: &str) -> Result<()> {
    let t0 = Instant::now();
    let scene = load_scene(input)?;
    let file_name = std::path::Path::new(input)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| input.to_string());
    eprintln!(
        "oxideplay: 3D viewer: {file_name} ({} meshes, {} triangles, {} animations) loaded in {:.0} ms",
        scene.meshes.len(),
        triangle_count(&scene),
        scene.animations.len(),
        t0.elapsed().as_secs_f64() * 1000.0
    );
    let title = format!("oxideplay — {file_name}");
    let mut fe = open_frontend(vo, &title)?;
    eprintln!("oxideplay: viewer output: {}", fe.describe());
    for (k, v) in HELP {
        eprintln!("  {k:<16} {v}");
    }
    let resolver = texture_resolver(input);
    run_loop(fe.as_mut(), scene, &file_name, resolver)
}

fn open_frontend(vo: &str, title: &str) -> Result<Box<dyn Frontend>> {
    match vo {
        #[cfg(feature = "winit")]
        "winit" => Ok(Box::new(winit_front::WinitFrontend::new(title)?)),
        #[cfg(feature = "sdl2")]
        "sdl2" => Ok(Box::new(sdl_front::SdlFrontend::new(title)?)),
        "auto" => {
            #[allow(unused_mut)]
            let mut errors: Vec<String> = Vec::new();
            #[cfg(feature = "winit")]
            match winit_front::WinitFrontend::new(title) {
                Ok(f) => return Ok(Box::new(f)),
                Err(e) => errors.push(format!("winit: {e}")),
            }
            #[cfg(feature = "sdl2")]
            match sdl_front::SdlFrontend::new(title) {
                Ok(f) => return Ok(Box::new(f)),
                Err(e) => errors.push(format!("sdl2: {e}")),
            }
            let _ = title;
            Err(Error::unsupported(format!(
                "viewer: no usable window output ({})",
                if errors.is_empty() {
                    "none compiled in".to_string()
                } else {
                    errors.join("; ")
                }
            )))
        }
        other => Err(Error::invalid(format!(
            "viewer: --vo '{other}' can't show a 3D model (use auto, winit or sdl2)"
        ))),
    }
}

fn run_loop(
    fe: &mut dyn Frontend,
    scene: Scene3D,
    file_name: &str,
    resolver: Arc<dyn TextureResolver>,
) -> Result<()> {
    let mut state =
        ViewerState::new(animation_duration(&scene)).with_scene_extent(scene_extent(&scene));
    state.viewport = fe.size();
    fe.set_texture_resolver(resolver.clone());
    let gpu_tris = fe.gpu_upload(&scene, &state.render_options(1, 1, 1));
    let triangles = gpu_tris.unwrap_or_else(|| triangle_count(&scene));
    let scene = Arc::new(scene);

    let backends = backend_list(gpu_tris.is_some(), software_backends());
    if backends.is_empty() {
        return Err(Error::unsupported("viewer: no render backend available"));
    }
    let mut backend_idx = 0;
    let gpu_adapter = fe.gpu_adapter();

    let mut worker = SoftWorker::spawn(scene.clone(), resolver);
    let mut generation: u64 = 0;
    let mut ladder: Vec<Pass> = Vec::new();
    let mut next_pass = 0usize;
    let mut preview_div: u32 = 2;
    let mut last_pass: Option<(Pass, Duration)> = None;
    let mut progress: Option<Progress> = None;
    let mut submitted_gen: u64 = 0;
    // GPU path tracer: refining until converged, samples per frame,
    // previous refining frame (for the spp adaptation), creation state.
    let mut gpu_pt_active = false;
    let mut gpu_pt_spp = GPU_PT_START_SPP;
    let mut gpu_pt_last: Option<Instant> = None;
    let mut gpu_pt_status: Option<GpuPtStatus> = None;
    let mut last_error: Option<String> = None;
    let mut reported_error: Option<String> = None;
    let mut fps = Fps::new();
    let mut last_tick = Instant::now();
    let mut hud_dirty = true;
    let mut gpu_posed_at: Option<f32> = state.animation.map(|a| a.time);

    loop {
        let backend = backends[backend_idx].clone();
        let soft_pending =
            matches!(backend, Backend::Soft(_)) && (worker.busy() || next_pass < ladder.len());
        let tracing = backend == Backend::Soft(PATHTRACE.into()) && worker.busy();
        let timeout = if backend == Backend::GpuPt && gpu_pt_active {
            if gpu_pt_status == Some(GpuPtStatus::Compiling) {
                Duration::from_millis(30)
            } else {
                // Presentation (FIFO) paces the refining frames.
                Duration::from_millis(1)
            }
        } else if state.animating() {
            Duration::from_millis(8)
        } else if tracing {
            // Progressive frames arrive every 80+ ms; don't spin.
            Duration::from_millis(16)
        } else if soft_pending {
            Duration::from_millis(4)
        } else {
            Duration::from_millis(250)
        };
        for ev in fe.pump(timeout) {
            match state.handle(ev) {
                Some(Command::Quit) => return Ok(()),
                Some(Command::ToggleFullscreen) => fe.toggle_fullscreen(),
                Some(Command::CycleBackend) => {
                    backend_idx = (backend_idx + 1) % backends.len();
                    last_pass = None;
                    progress = None;
                    last_error = None;
                    gpu_pt_last = None;
                    state.invalidate();
                }
                None => {}
            }
        }
        let backend = backends[backend_idx].clone();
        // Mouse deltas are in drawable pixels; keep the pan scale in
        // step with the window (fullscreen toggles, DPI changes).
        state.viewport = fe.size();

        let now = Instant::now();
        let dt = now.duration_since(last_tick).as_secs_f32().min(0.25);
        last_tick = now;
        state.tick(dt);

        if state.take_dirty() {
            generation += 1;
            hud_dirty = true;
            let (w, h) = fe.size();
            match &backend {
                Backend::Gpu => {
                    let aa = if state.aa { AA_LEVEL } else { 1 };
                    let opts = state.render_options(w, h, aa);
                    let t = Instant::now();
                    // The GPU poses the scene at upload time: re-upload
                    // whenever the animation clock moved.
                    if opts.time.is_some() && opts.time != gpu_posed_at {
                        fe.gpu_upload(&scene, &opts);
                        gpu_posed_at = opts.time;
                    }
                    match fe.gpu_draw(&opts) {
                        Ok(()) => {
                            last_pass = Some((Pass::new(1, aa), t.elapsed()));
                            fps.frame();
                        }
                        Err(e) => last_error = Some(e.to_string()),
                    }
                    ladder.clear();
                    next_pass = 0;
                }
                Backend::GpuPt => {
                    gpu_pt_active = true;
                    ladder.clear();
                    next_pass = 0;
                }
                Backend::Soft(name) => {
                    let interacting = state.interacting() || state.animating();
                    let aa = if state.aa { AA_LEVEL } else { 1 };
                    ladder = plan_passes(name, interacting, preview_div, aa);
                    next_pass = 0;
                }
            }
        }

        if backend == Backend::GpuPt && gpu_pt_active {
            let status = fe
                .gpu_pt_poll()
                .unwrap_or_else(|| GpuPtStatus::Failed("no GPU path tracer".into()));
            if gpu_pt_status.as_ref() != Some(&status) {
                hud_dirty = true;
            }
            gpu_pt_status = Some(status.clone());
            match status {
                GpuPtStatus::Compiling => {}
                GpuPtStatus::Failed(e) => {
                    last_error = Some(e);
                    gpu_pt_active = false;
                }
                GpuPtStatus::Ready => {
                    let (w, h) = fe.size();
                    let opts = state.render_options(w, h, 1);
                    let t = Instant::now();
                    match fe.gpu_pt_frame(&scene, &opts, gpu_pt_spp) {
                        Ok(p) => {
                            let now = Instant::now();
                            if let Some(prev) = gpu_pt_last {
                                gpu_pt_spp = adapt_gpu_spp(gpu_pt_spp, now - prev);
                            }
                            fps.frame();
                            last_pass = Some((Pass::new(1, 1), t.elapsed()));
                            last_error = None;
                            progress = Some(p);
                            if p.samples >= p.target && !state.animating() {
                                gpu_pt_active = false;
                                gpu_pt_last = None;
                            } else {
                                gpu_pt_last = Some(now);
                            }
                        }
                        Err(e) => {
                            last_error = Some(e.to_string());
                            gpu_pt_active = false;
                            gpu_pt_last = None;
                        }
                    }
                    hud_dirty = true;
                }
            }
        }

        if let Backend::Soft(name) = &backend {
            // A progressive pass runs until converged: a newer
            // generation preempts it rather than waiting.
            let free = !worker.busy() || (preemptible(name) && submitted_gen < generation);
            if free && next_pass < ladder.len() {
                let pass = ladder[next_pass];
                submitted_gen = generation;
                next_pass += 1;
                let (w, h) = fe.size();
                let opts = state.render_options(
                    w.div_ceil(pass.div).max(1),
                    h.div_ceil(pass.div).max(1),
                    pass.aa,
                );
                worker.submit(generation, pass, name, opts);
            }
        }

        if let Some(frame) = worker.try_recv() {
            let current = Backend::Soft(frame.backend.clone()) == backend;
            match frame.image {
                Ok(img) if current => {
                    fe.set_image(&img)?;
                    fps.frame();
                    last_pass = Some((frame.pass, frame.elapsed));
                    progress = frame.progress;
                    last_error = None;
                    if frame.generation == generation && frame.pass.div > 1 {
                        preview_div = adapt_preview_div(preview_div, frame.elapsed);
                    }
                    hud_dirty = true;
                }
                Ok(_) => {}
                Err(e) => {
                    last_error = Some(e);
                    hud_dirty = true;
                }
            }
        }

        if last_error != reported_error {
            if let Some(e) = last_error.as_deref() {
                eprintln!("oxideplay: viewer: {} backend: {e}", backend.label());
            }
            reported_error = last_error.clone();
        }

        if hud_dirty {
            hud_dirty = false;
            let hud = build_hud(
                &state,
                &backend,
                gpu_adapter.as_deref(),
                file_name,
                triangles,
                fps.value,
                last_pass,
                progress,
                (backend == Backend::GpuPt && gpu_pt_status == Some(GpuPtStatus::Compiling))
                    .then_some("compiling shaders…"),
                last_error.as_deref(),
            );
            fe.present(&hud)?;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn build_hud(
    state: &ViewerState,
    backend: &Backend,
    gpu_adapter: Option<&str>,
    file_name: &str,
    triangles: usize,
    fps: f32,
    last_pass: Option<(Pass, Duration)>,
    progress: Option<Progress>,
    status: Option<&str>,
    last_error: Option<&str>,
) -> Hud {
    let mode = shading_name(state.shading_mode());
    let proj = match state.projection {
        oxideav_render::Projection::Orthographic => "ortho",
        _ => "persp",
    };
    let backend_label = match (backend, gpu_adapter) {
        (Backend::Gpu, Some(a)) => format!("gpu — {a}"),
        (Backend::GpuPt, Some(a)) => format!("gpu path trace — {a}"),
        (b, _) => format!("{} (software)", b.label()),
    };
    let mut rows = vec![
        ("file".to_string(), file_name.to_string()),
        ("backend".to_string(), backend_label),
        ("triangles".to_string(), triangles.to_string()),
        ("fps".to_string(), format!("{fps:.1}")),
    ];
    if let Some((pass, t)) = last_pass {
        let res = if pass.div > 1 {
            format!("1/{} res", pass.div)
        } else {
            "full res".to_string()
        };
        rows.push((
            "last frame".to_string(),
            format!("{:.1} ms, {res}, aa {}", t.as_secs_f64() * 1000.0, pass.aa),
        ));
    }
    if let Some(s) = status {
        rows.push(("status".to_string(), s.to_string()));
    }
    if let Some(p) = progress {
        let pct = 100.0 * p.samples as f64 / p.target.max(1) as f64;
        rows.push((
            "samples".to_string(),
            if p.samples >= p.target {
                format!("{} spp (converged)", p.samples)
            } else {
                format!("{} / {} spp ({pct:.1}%)", p.samples, p.target)
            },
        ));
    }
    if matches!(backend, Backend::GpuPt) || *backend == Backend::Soft(PATHTRACE.into()) {
        rows.push((
            "path trace".to_string(),
            format!(
                "gi {}, ambient {:.2}",
                if state.gi { "on" } else { "off (direct only)" },
                state.ambient
            ),
        ));
    }
    rows.push((
        "mode".to_string(),
        format!("{mode}, {proj}, aa {}", if state.aa { "on" } else { "off" }),
    ));
    rows.push((
        "tone".to_string(),
        format!(
            "{}, exposure {:.2}",
            state::tone_map_name(state.tone_map),
            state.exposure
        ),
    ));
    rows.push((
        "camera".to_string(),
        format!(
            "az {:.0}° el {:.0}° dist {:.2}{}",
            state.azimuth,
            state.elevation,
            state.distance,
            if state.pan == [0.0; 3] {
                String::new()
            } else {
                format!(
                    " pan ({:.2}, {:.2}, {:.2})",
                    state.pan[0], state.pan[1], state.pan[2]
                )
            }
        ),
    ));
    rows.push((
        "light".to_string(),
        format!(
            "az {:.0}° el {:.0}°{}",
            state.light_azimuth,
            state.light_elevation,
            if state.light_mode { " (drag)" } else { "" }
        ),
    ));
    if let Some(a) = state.animation {
        let v = format!(
            "{} t={:.2}s",
            if a.playing { "playing" } else { "paused" },
            a.time
        );
        rows.push(("animation".to_string(), v));
    }
    if state.turntable {
        rows.push(("turntable".to_string(), "on".to_string()));
    }
    if let Some(e) = last_error {
        rows.push(("error".to_string(), e.to_string()));
    }
    Hud {
        visible: state.show_hud,
        summary: format!(
            "oxideplay — {file_name} — {} — {mode} — {fps:.0} fps",
            backend.label()
        ),
        rows,
    }
}

/// Procedural test scenes shared by the viewer's unit tests.
#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;

    /// A unit cube as ASCII STL — a small procedural scene decoded
    /// through the same registry path the viewer uses for real files.
    fn cube_stl() -> String {
        let v = |x: i32, y: i32, z: i32| format!("vertex {x} {y} {z}\n");
        let quads: [[(i32, i32, i32); 4]; 6] = [
            [(0, 0, 0), (1, 0, 0), (1, 1, 0), (0, 1, 0)],
            [(0, 0, 1), (0, 1, 1), (1, 1, 1), (1, 0, 1)],
            [(0, 0, 0), (0, 0, 1), (1, 0, 1), (1, 0, 0)],
            [(0, 1, 0), (1, 1, 0), (1, 1, 1), (0, 1, 1)],
            [(0, 0, 0), (0, 1, 0), (0, 1, 1), (0, 0, 1)],
            [(1, 0, 0), (1, 0, 1), (1, 1, 1), (1, 1, 0)],
        ];
        let mut s = String::from("solid cube\n");
        for q in quads {
            for tri in [[q[0], q[1], q[2]], [q[0], q[2], q[3]]] {
                s.push_str("facet normal 0 0 0\nouter loop\n");
                for (x, y, z) in tri {
                    s.push_str(&v(x, y, z));
                }
                s.push_str("endloop\nendfacet\n");
            }
        }
        s.push_str("endsolid cube\n");
        s
    }

    /// Unit square in the z = 0 plane: it lies in the orbit target's
    /// plane, so screen motion there is exactly 1:1 with the pan.
    pub fn quad_scene() -> Scene3D {
        scene_from_stl(
            "solid q\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nvertex 1 1 0\nendloop\nendfacet\n\
             facet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 1 1 0\nvertex 0 1 0\nendloop\nendfacet\nendsolid q\n",
        )
    }

    pub fn cube_scene() -> Scene3D {
        scene_from_stl(&cube_stl())
    }

    fn scene_from_stl(stl: &str) -> Scene3D {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("oxideplay-viewer-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cube.stl");
        std::fs::write(&path, stl).unwrap();
        let p = path.to_string_lossy().into_owned();
        assert!(is_model_input(&p));
        let scene = load_scene(&p).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        scene
    }
}

#[cfg(test)]
mod tests {
    use super::tests_support::cube_scene;
    use super::*;

    #[test]
    fn detects_model_inputs_by_registry_extension() {
        assert!(is_model_input("model.STL"));
        assert!(is_model_input("file:///tmp/a.obj"));
        assert!(is_model_input("scene.glb"));
        assert!(is_model_input("scene.gltf"));
        assert!(!is_model_input("movie.mkv"));
        assert!(!is_model_input("song.flac"));
        assert!(!is_model_input("https://example.com/model.glb"));
        assert!(!is_model_input("noext"));
    }

    #[test]
    fn cube_scene_stats() {
        let scene = cube_scene();
        assert_eq!(triangle_count(&scene), 12);
        assert_eq!(animation_duration(&scene), None);
    }

    /// The software path produces a frame for a small procedural scene
    /// through the worker thread, honouring the requested size, and the
    /// model actually covers pixels (not just background).
    #[test]
    fn software_path_renders_a_frame() {
        let scene = Arc::new(cube_scene());
        let mut worker = SoftWorker::spawn(scene, texture_resolver("cube.stl"));
        let state = ViewerState::default();
        for name in software_backends() {
            let pass = Pass::new(1, 1);
            worker.submit(1, pass, &name, state.render_options(64, 48, 1));
            assert!(worker.busy());
            let frame = worker
                .recv_timeout(Duration::from_secs(60))
                .expect("worker answered");
            assert_eq!(frame.generation, 1);
            assert_eq!(frame.backend, name);
            let img = frame.image.expect("rendered");
            assert_eq!((img.width, img.height), (64, 48));
            let bg = state::BACKGROUND;
            let covered = img.pixels_rgba().filter(|p| *p != bg).count();
            assert!(covered > 64 * 48 / 20, "{name}: cube covers {covered} px");
        }
    }

    /// Centroid x and covered-pixel count of the non-background pixels.
    fn coverage(img: &RgbaImage) -> (f64, usize) {
        let bg = state::BACKGROUND;
        let (mut sx, mut n) = (0.0, 0usize);
        for y in 0..img.height {
            for x in 0..img.width {
                if img.pixel(x, y) != Some(bg) {
                    sx += x as f64;
                    n += 1;
                }
            }
        }
        (sx / n.max(1) as f64, n)
    }

    /// Pan moves the model 1:1 with the cursor (perspective and ortho),
    /// and ortho zoom scales the image — rendered through scanline.
    #[test]
    fn pan_is_one_to_one_and_ortho_zooms() {
        use state::{InputEvent, Key, MouseButton};
        let scene = super::tests_support::quad_scene();
        let mut r = oxideav_render::make_renderer(oxideav_render::RenderBackend::Scanline).unwrap();
        let (w, h) = (160u32, 120u32);
        for ortho in [false, true] {
            let mut s = ViewerState::default().with_scene_extent(scene_extent(&scene));
            s.handle(InputEvent::Resized(w, h));
            // Face-on so a screen-x pan is a pure image translation.
            s.azimuth = 0.0;
            s.elevation = 0.0;
            s.handle(InputEvent::Key(Key::Char('3')));
            if ortho {
                s.handle(InputEvent::Key(Key::Char('p')));
            }
            let (x0, n0) = coverage(&r.render(&scene, &s.render_options(w, h, 1)).unwrap());
            assert!(n0 > 0);
            s.handle(InputEvent::MouseDown {
                button: MouseButton::Right,
                x: 50.0,
                y: 50.0,
            });
            s.handle(InputEvent::MouseMove { x: 70.0, y: 50.0 });
            s.handle(InputEvent::MouseUp {
                button: MouseButton::Right,
            });
            let (x1, n1) = coverage(&r.render(&scene, &s.render_options(w, h, 1)).unwrap());
            let shift = x1 - x0;
            assert!(
                (shift - 20.0).abs() < 2.0,
                "ortho={ortho}: 20 px drag moved the model {shift:.2} px"
            );
            if ortho {
                // Zooming in scales the model by 1 / distance.
                let width = |img: &RgbaImage| {
                    let bg = state::BACKGROUND;
                    (0..w)
                        .filter(|x| (0..h).any(|y| img.pixel(*x, y) != Some(bg)))
                        .count() as f32
                };
                s.pan = [0.0; 3];
                let w1 = width(&r.render(&scene, &s.render_options(w, h, 1)).unwrap());
                s.handle(InputEvent::Wheel(2.0));
                let w2 = width(&r.render(&scene, &s.render_options(w, h, 1)).unwrap());
                let ratio = w2 / w1;
                let want = 1.0 / s.distance;
                assert!(
                    (ratio - want).abs() < 0.05,
                    "ortho zoom: width {w1} → {w2} (×{ratio:.3}, want ×{want:.3})"
                );
                let _ = n1;
            }
        }
    }

    /// Scripted headless window driven by what the viewer shows: it
    /// selects the path tracer, waits for idle refinement, drags (one
    /// motion per pump), releases, waits for refinement to resume, and
    /// quits. Records every frame size and HUD.
    struct FakeFrontend {
        b_presses: usize,
        stage: u32,
        moves: u32,
        /// HUD index where the drag began.
        drag_mark: usize,
        deadline: Instant,
        images: Vec<(u32, u32)>,
        huds: Vec<Hud>,
    }

    fn spp(h: &Hud) -> Option<u32> {
        if !h.summary.contains(PATHTRACE) {
            return None;
        }
        h.rows
            .iter()
            .find(|(k, _)| k == "samples")
            .and_then(|(_, v)| v.split_whitespace().next()?.parse().ok())
    }

    impl Frontend for FakeFrontend {
        fn pump(&mut self, timeout: Duration) -> Vec<state::InputEvent> {
            use state::{InputEvent as E, Key, MouseButton};
            std::thread::sleep(timeout.min(Duration::from_millis(5)));
            assert!(
                Instant::now() < self.deadline,
                "stalled at stage {}",
                self.stage
            );
            let latest = self.huds.iter().rev().find_map(spp);
            match self.stage {
                0 => {
                    self.stage = 1;
                    (0..self.b_presses)
                        .map(|_| E::Key(Key::Char('b')))
                        .collect()
                }
                1 if latest.is_some_and(|n| n >= 4) => {
                    self.stage = 2;
                    self.drag_mark = self.huds.len();
                    vec![E::MouseDown {
                        button: MouseButton::Left,
                        x: 0.0,
                        y: 0.0,
                    }]
                }
                2 => {
                    self.moves += 1;
                    if self.moves > 6 {
                        self.stage = 3;
                        vec![E::MouseUp {
                            button: MouseButton::Left,
                        }]
                    } else {
                        std::thread::sleep(Duration::from_millis(30));
                        vec![E::MouseMove {
                            x: self.moves as f64 * 6.0,
                            y: 0.0,
                        }]
                    }
                }
                // Any 1-spp frame since the drag began (several HUDs can
                // land between two pumps).
                3 if self.huds[self.drag_mark..]
                    .iter()
                    .any(|h| spp(h) == Some(1)) =>
                {
                    self.stage = 4;
                    Vec::new()
                }
                4 if latest.is_some_and(|n| n >= 4) => {
                    self.stage = 5;
                    vec![E::Key(Key::Char('q'))]
                }
                _ => Vec::new(),
            }
        }
        fn size(&self) -> (u32, u32) {
            (48, 36)
        }
        fn set_image(&mut self, img: &RgbaImage) -> Result<()> {
            self.images.push((img.width, img.height));
            Ok(())
        }
        fn present(&mut self, hud: &Hud) -> Result<()> {
            self.huds.push(hud.clone());
            Ok(())
        }
        fn toggle_fullscreen(&mut self) {}
        fn describe(&self) -> String {
            "fake".into()
        }
    }

    /// The real viewer loop, headless: cycle to the path tracer, let it
    /// refine, drag (1-spp reduced-res previews preempt the progressive
    /// pass), release (refines again from a reset), then quit.
    #[test]
    fn viewer_loop_drives_the_path_tracer_progressively() {
        let backends = software_backends();
        let mut fe = FakeFrontend {
            b_presses: backends.iter().position(|b| b == PATHTRACE).unwrap(),
            stage: 0,
            moves: 0,
            drag_mark: 0,
            deadline: Instant::now() + Duration::from_secs(120),
            images: Vec::new(),
            huds: Vec::new(),
        };
        run_loop(
            &mut fe,
            cube_scene(),
            "cube.stl",
            texture_resolver("cube.stl"),
        )
        .unwrap();
        assert_eq!(fe.stage, 5);
        let counts: Vec<u32> = fe.huds.iter().filter_map(spp).collect();
        // Idle refinement climbs, the drag resets to 1 spp, refinement
        // resumes after release.
        let climbed = counts.iter().position(|c| *c >= 4).unwrap();
        let reset = climbed + counts[climbed..].iter().position(|c| *c == 1).unwrap();
        assert!(counts[reset..].iter().any(|c| *c >= 4), "{counts:?}");
        // Previews were reduced resolution, refinement full resolution.
        assert!(fe.images.iter().any(|&(w, _)| w < 48), "{:?}", fe.images);
        assert!(fe.images.iter().any(|&(w, _)| w == 48));
        assert!(fe.huds.iter().any(|h| h
            .rows
            .iter()
            .any(|(k, v)| k == "path trace" && v.contains("gi on"))));
    }

    #[test]
    fn backend_cycle_order_and_no_gpu_fallback() {
        let soft = || vec!["scanline".to_string(), "raycast".into(), PATHTRACE.into()];
        let labels = |b: Vec<Backend>| b.iter().map(Backend::label).collect::<Vec<_>>();
        assert_eq!(
            labels(backend_list(true, soft())),
            ["gpu", "gpu-pathtrace", "scanline", "raycast", "pathtrace"]
        );
        assert_eq!(
            labels(backend_list(false, soft())),
            ["scanline", "raycast", "pathtrace"]
        );
        let compiling = build_hud(
            &ViewerState::default(),
            &Backend::GpuPt,
            Some("Test GPU"),
            "cube.stl",
            12,
            0.0,
            None,
            None,
            Some("compiling shaders…"),
            None,
        );
        let row = |k: &str| {
            compiling
                .rows
                .iter()
                .find(|(l, _)| l == k)
                .map(|(_, v)| v.clone())
        };
        assert_eq!(row("status").as_deref(), Some("compiling shaders…"));
        assert!(row("backend").unwrap().contains("gpu path trace"));
        assert!(row("path trace").is_some());
    }

    #[test]
    fn hud_reports_state() {
        let state = ViewerState::new(Some(Some(1.0)));
        let hud = build_hud(
            &state,
            &Backend::Gpu,
            Some("Test GPU"),
            "cube.stl",
            12,
            60.0,
            Some((Pass::new(2, 1), Duration::from_millis(5))),
            Some(Progress {
                samples: 37,
                target: 1024,
                reset: false,
            }),
            None,
            None,
        );
        assert!(hud.visible);
        let get = |k: &str| {
            hud.rows
                .iter()
                .find(|(l, _)| l == k)
                .map(|(_, v)| v.clone())
                .unwrap_or_default()
        };
        assert!(get("backend").contains("Test GPU"));
        assert_eq!(get("triangles"), "12");
        assert!(get("last frame").contains("1/2 res"));
        assert!(get("samples").starts_with("37 / 1024 spp"));
        assert!(get("animation").contains("playing"));
        assert!(hud.summary.contains("gpu"));
    }
}

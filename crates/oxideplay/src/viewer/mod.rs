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

use self::soft::{adapt_preview_div, plan_passes, software_backends, Pass, SoftWorker};
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
    Gpu,
    Soft(String),
}

impl Backend {
    fn label(&self) -> String {
        match self {
            Backend::Gpu => "gpu".into(),
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

    let mut backends: Vec<Backend> = Vec::new();
    if gpu_tris.is_some() {
        backends.push(Backend::Gpu);
    }
    backends.extend(software_backends().into_iter().map(Backend::Soft));
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
        let timeout = if state.animating() {
            Duration::from_millis(8)
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
                    last_error = None;
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
                            last_pass = Some((Pass { div: 1, aa }, t.elapsed()));
                            fps.frame();
                        }
                        Err(e) => last_error = Some(e.to_string()),
                    }
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

        if let Backend::Soft(name) = &backend {
            if !worker.busy() && next_pass < ladder.len() {
                let pass = ladder[next_pass];
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
    last_error: Option<&str>,
) -> Hud {
    let mode = shading_name(state.shading_mode());
    let proj = match state.projection {
        oxideav_render::Projection::Orthographic => "ortho",
        _ => "persp",
    };
    let backend_label = match (backend, gpu_adapter) {
        (Backend::Gpu, Some(a)) => format!("gpu — {a}"),
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
            let pass = Pass { div: 1, aa: 1 };
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
            Some((Pass { div: 2, aa: 1 }, Duration::from_millis(5))),
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
        assert!(get("animation").contains("playing"));
        assert!(hud.summary.contains("gpu"));
    }
}

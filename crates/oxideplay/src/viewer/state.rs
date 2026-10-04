//! Window-independent viewer state: orbit camera, light, shading /
//! projection / AA toggles, animation clock, and the input state
//! machine that drives them.
//!
//! Every frontend (winit, SDL2) translates its native events into
//! [`InputEvent`]s and feeds them to [`ViewerState::handle`]; nothing
//! in here touches a window, so the whole input surface is unit
//! testable headlessly.

use oxideav_render::{
    BackgroundColor, CameraSpec, LightSpec, PathTraceOptions, Projection, RenderOptions,
    ShadingMode, ToneMap,
};

/// Elevation clamp (degrees) — keeps the orbit off the poles where the
/// renderer's fixed world-up would make `look_at` degenerate.
pub const MAX_ELEVATION: f32 = 89.0;
/// Zoom range, as a multiple of the auto-frame distance.
pub const MIN_DISTANCE: f32 = 0.15;
pub const MAX_DISTANCE: f32 = 20.0;
/// Multiplicative zoom step per wheel notch / `+` / `-` press.
pub const ZOOM_STEP: f32 = 1.12;
/// Degrees of orbit per pixel of mouse drag.
pub const DRAG_DEG_PER_PX: f32 = 0.4;
/// Turntable speed (degrees per second).
pub const TURNTABLE_DEG_PER_SEC: f32 = 30.0;
/// Supersampling factor used when AA is on and the view is idle.
pub const AA_LEVEL: u32 = 4;
/// Exposure step per `[` / `]` press (half a stop).
pub const EXPOSURE_STEP: f32 = std::f32::consts::SQRT_2;
/// Exposure range (linear multiplier).
pub const MIN_EXPOSURE: f32 = 1.0 / 64.0;
pub const MAX_EXPOSURE: f32 = 64.0;

/// Ambient (constant sky) range and step for `,` / `.`.
pub const MAX_AMBIENT: f32 = 4.0;
pub const AMBIENT_STEP: f32 = 1.4;
const DEFAULT_AMBIENT: f32 = 0.2;

/// Tone-map operators reachable with `T`, in cycle order.
pub const TONE_MAPS: [ToneMap; 3] = [ToneMap::Clamp, ToneMap::Reinhard, ToneMap::AcesFitted];

/// Human label for a tone-map operator.
pub fn tone_map_name(t: ToneMap) -> &'static str {
    match t {
        ToneMap::Clamp => "clamp",
        ToneMap::Reinhard => "reinhard",
        ToneMap::AcesFitted => "aces",
        #[allow(unreachable_patterns)]
        _ => "other",
    }
}

const DEFAULT_AZIMUTH: f32 = 35.0;
const DEFAULT_ELEVATION: f32 = 25.0;
const DEFAULT_DISTANCE: f32 = 1.0;
/// Default shading: index of `Pbr` in [`SHADING_MODES`] (every backend
/// accepts it; those without a PBR path fall back to Phong).
const DEFAULT_SHADING: usize = 3;

/// Viewer background: dark neutral grey, opaque so the window never
/// shows garbage through transparent pixels.
pub const BACKGROUND: [u8; 4] = [38, 40, 46, 255];

/// Shading modes reachable from the keyboard, in `1`..`7` order.
pub const SHADING_MODES: [ShadingMode; 7] = [
    ShadingMode::Flat,
    ShadingMode::Gouraud,
    ShadingMode::Phong,
    ShadingMode::Pbr,
    ShadingMode::Wireframe,
    ShadingMode::NormalDebug,
    ShadingMode::DepthDebug,
];

/// Human label for a shading mode (HUD / window title).
pub fn shading_name(mode: ShadingMode) -> &'static str {
    match mode {
        ShadingMode::Flat => "flat",
        ShadingMode::Gouraud => "gouraud",
        ShadingMode::Phong => "phong",
        ShadingMode::Pbr => "pbr",
        ShadingMode::Wireframe => "wireframe",
        ShadingMode::NormalDebug => "normals",
        ShadingMode::DepthDebug => "depth",
        #[allow(unreachable_patterns)]
        _ => "other",
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

/// Window-independent key identity. Letters are normalised to
/// lowercase by the frontends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Home,
    Escape,
    F1,
}

/// One input event, already translated out of the native toolkit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum InputEvent {
    Key(Key),
    /// Current Shift state (frontends emit this whenever modifiers
    /// change; it decides whether a left-drag moves the light).
    Shift(bool),
    MouseDown {
        button: MouseButton,
        x: f64,
        y: f64,
    },
    MouseUp {
        button: MouseButton,
    },
    MouseMove {
        x: f64,
        y: f64,
    },
    /// Wheel notches, positive = away from the user = zoom in.
    Wheel(f32),
    Resized(u32, u32),
    CloseRequested,
}

/// Side effects the state machine can't perform itself — the main
/// loop / frontend acts on them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Quit,
    ToggleFullscreen,
    CycleBackend,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DragKind {
    Orbit,
    Light,
    Pan,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Drag {
    button: MouseButton,
    kind: DragKind,
    last: (f64, f64),
}

/// The viewer's whole interactive state.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewerState {
    pub azimuth: f32,
    pub elevation: f32,
    pub distance: f32,
    pub light_azimuth: f32,
    pub light_elevation: f32,
    pub shading: usize,
    pub projection: Projection,
    pub aa: bool,
    /// Linear exposure multiplier (before tone mapping).
    pub exposure: f32,
    pub tone_map: ToneMap,
    /// Constant ambient / sky radiance (Pbr shading and the path
    /// tracer's environment).
    pub ambient: f32,
    /// Path tracer global illumination: full bounce depth when on,
    /// direct lighting only (`max_bounces = 1`) when off.
    pub gi: bool,
    /// Turntable auto-rotation.
    pub turntable: bool,
    /// `L` latches left-drag onto the light (Shift+drag does it
    /// momentarily).
    pub light_mode: bool,
    pub show_hud: bool,
    /// World-space offset of the orbit target (right / middle drag).
    pub pan: [f32; 3],
    /// Largest axis of the scene's bounding box (world units) — the
    /// renderer frames the view on it, so panning uses it to move the
    /// model 1:1 with the cursor.
    pub scene_extent: f32,
    /// Drawable size in pixels (the space mouse coordinates live in).
    pub viewport: (u32, u32),
    /// Scene animation clock. `None` when the scene has no animations.
    pub animation: Option<AnimationClock>,
    shift: bool,
    drag: Option<Drag>,
    dirty: bool,
}

/// Playback state for the scene's first animation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnimationClock {
    pub playing: bool,
    pub time: f32,
    /// Loop length in seconds (`None` / non-positive: unbounded).
    pub duration: Option<f32>,
}

impl Default for ViewerState {
    fn default() -> Self {
        Self::new(None)
    }
}

impl ViewerState {
    /// Fresh state. `animation_duration` is `Some` when the scene has
    /// at least one animation (its length in seconds, if known);
    /// playback starts running.
    pub fn new(animation_duration: Option<Option<f32>>) -> Self {
        let light = LightSpec::default_light();
        Self {
            azimuth: DEFAULT_AZIMUTH,
            elevation: DEFAULT_ELEVATION,
            distance: DEFAULT_DISTANCE,
            light_azimuth: light.azimuth_deg,
            light_elevation: light.elevation_deg,
            shading: DEFAULT_SHADING,
            projection: Projection::Perspective,
            aa: true,
            exposure: 1.0,
            tone_map: ToneMap::Clamp,
            ambient: DEFAULT_AMBIENT,
            gi: true,
            turntable: false,
            light_mode: false,
            show_hud: true,
            pan: [0.0; 3],
            scene_extent: 1.0,
            viewport: (1, 1),
            animation: animation_duration.map(|duration| AnimationClock {
                playing: true,
                time: 0.0,
                duration,
            }),
            shift: false,
            drag: None,
            dirty: true,
        }
    }

    /// Set the scene's largest bounding-box axis (pan scale).
    pub fn with_scene_extent(mut self, extent: f32) -> Self {
        self.scene_extent = extent;
        self
    }

    pub fn shading_mode(&self) -> ShadingMode {
        SHADING_MODES[self.shading % SHADING_MODES.len()]
    }

    /// True while a mouse drag is in progress — the software path
    /// renders reduced-resolution previews during it.
    pub fn interacting(&self) -> bool {
        self.drag.is_some()
    }

    /// True when the view changes on its own every frame (turntable or
    /// a playing animation), so the loop must keep redrawing.
    pub fn animating(&self) -> bool {
        self.turntable || self.animation.is_some_and(|a| a.playing)
    }

    /// Consume the "needs redraw" flag.
    pub fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    /// Mark the view dirty (e.g. after a resize or backend switch).
    pub fn invalidate(&mut self) {
        self.dirty = true;
    }

    /// Restore the initial camera + light (R / Home).
    pub fn reset_view(&mut self) {
        let light = LightSpec::default_light();
        self.azimuth = DEFAULT_AZIMUTH;
        self.elevation = DEFAULT_ELEVATION;
        self.distance = DEFAULT_DISTANCE;
        self.light_azimuth = light.azimuth_deg;
        self.light_elevation = light.elevation_deg;
        self.pan = [0.0; 3];
        self.dirty = true;
    }

    fn zoom(&mut self, notches: f32) {
        if notches == 0.0 || !notches.is_finite() {
            return;
        }
        self.distance = (self.distance / ZOOM_STEP.powf(notches)).clamp(MIN_DISTANCE, MAX_DISTANCE);
        self.dirty = true;
    }

    fn set_exposure(&mut self, e: f32) {
        self.exposure = e.clamp(MIN_EXPOSURE, MAX_EXPOSURE);
        self.dirty = true;
    }

    /// Camera basis `(side, up)` for the current orbit — matches the
    /// renderer's `look_at` with world-up `+Y`.
    pub fn camera_basis(&self) -> ([f32; 3], [f32; 3]) {
        let (el, az) = (self.elevation.to_radians(), self.azimuth.to_radians());
        let dir = [el.cos() * az.sin(), el.sin(), el.cos() * az.cos()];
        let fwd = [-dir[0], -dir[1], -dir[2]];
        // side = normalize(forward × +Y) = normalize((-fz, 0, fx)).
        let len = (fwd[0] * fwd[0] + fwd[2] * fwd[2]).sqrt().max(1e-6);
        let side = [-fwd[2] / len, 0.0, fwd[0] / len];
        // up = side × forward.
        let up = [
            side[1] * fwd[2] - side[2] * fwd[1],
            side[2] * fwd[0] - side[0] * fwd[2],
            side[0] * fwd[1] - side[1] * fwd[0],
        ];
        (side, up)
    }

    /// World units per screen pixel at the orbit target, mirroring the
    /// renderer's framing (`radius = extent * 0.6`; perspective half
    /// height `radius * distance`, orthographic `radius * distance`
    /// on the shorter axis).
    pub fn world_per_pixel(&self) -> f32 {
        let (w, h) = (self.viewport.0.max(1) as f32, self.viewport.1.max(1) as f32);
        let radius = self.scene_extent.max(1e-3) * 0.6;
        let mut half_h = radius * self.distance;
        if self.projection == Projection::Orthographic && w < h {
            half_h /= w / h;
        }
        2.0 * half_h / h
    }

    fn pan_by(&mut self, dx: f64, dy: f64) {
        let (side, up) = self.camera_basis();
        let k = self.world_per_pixel();
        let (dx, dy) = (dx as f32 * k, dy as f32 * k);
        // Dragging right moves the model right → the target moves left;
        // dragging down (screen y grows) moves the target up.
        for (i, p) in self.pan.iter_mut().enumerate() {
            *p += -side[i] * dx + up[i] * dy;
        }
        self.dirty = true;
    }

    fn orbit(&mut self, dx: f64, dy: f64) {
        // Dragging right spins the model right: the camera moves the
        // other way round the orbit.
        self.azimuth = wrap_degrees(self.azimuth - dx as f32 * DRAG_DEG_PER_PX);
        self.elevation =
            (self.elevation + dy as f32 * DRAG_DEG_PER_PX).clamp(-MAX_ELEVATION, MAX_ELEVATION);
        self.dirty = true;
    }

    fn move_light(&mut self, dx: f64, dy: f64) {
        self.light_azimuth = wrap_degrees(self.light_azimuth + dx as f32 * DRAG_DEG_PER_PX);
        self.light_elevation = (self.light_elevation - dy as f32 * DRAG_DEG_PER_PX)
            .clamp(-MAX_ELEVATION, MAX_ELEVATION);
        self.dirty = true;
    }

    /// Feed one input event. Returns a [`Command`] when the event asks
    /// for something outside the state (quit, fullscreen, backend).
    pub fn handle(&mut self, ev: InputEvent) -> Option<Command> {
        match ev {
            InputEvent::CloseRequested => return Some(Command::Quit),
            InputEvent::Resized(w, h) => {
                self.viewport = (w.max(1), h.max(1));
                self.dirty = true;
            }
            InputEvent::Shift(s) => self.shift = s,
            InputEvent::Wheel(n) => self.zoom(n),
            InputEvent::MouseDown { button, x, y } => {
                let kind = match button {
                    MouseButton::Left if self.shift || self.light_mode => Some(DragKind::Light),
                    MouseButton::Left => Some(DragKind::Orbit),
                    MouseButton::Right | MouseButton::Middle => Some(DragKind::Pan),
                };
                if let Some(kind) = kind {
                    self.drag = Some(Drag {
                        button,
                        kind,
                        last: (x, y),
                    });
                }
            }
            InputEvent::MouseUp { button } => {
                if self.drag.is_some_and(|d| d.button == button) {
                    self.drag = None;
                    // Release → refine at full quality.
                    self.dirty = true;
                }
            }
            InputEvent::MouseMove { x, y } => {
                if let Some(d) = self.drag.as_mut() {
                    let (dx, dy) = (x - d.last.0, y - d.last.1);
                    d.last = (x, y);
                    let kind = d.kind;
                    if dx != 0.0 || dy != 0.0 {
                        match kind {
                            DragKind::Orbit => self.orbit(dx, dy),
                            DragKind::Light => self.move_light(dx, dy),
                            DragKind::Pan => self.pan_by(dx, dy),
                        }
                    }
                }
            }
            InputEvent::Key(k) => return self.key(k),
        }
        None
    }

    fn key(&mut self, k: Key) -> Option<Command> {
        match k {
            Key::Escape | Key::Char('q') => return Some(Command::Quit),
            Key::Char('f') => return Some(Command::ToggleFullscreen),
            Key::Char('b') => return Some(Command::CycleBackend),
            Key::Home | Key::Char('r') => self.reset_view(),
            Key::Char('+') | Key::Char('=') => self.zoom(1.0),
            Key::Char('-') | Key::Char('_') => self.zoom(-1.0),
            Key::Char(c @ '1'..='7') => {
                self.shading = (c as usize - '1' as usize) % SHADING_MODES.len();
                self.dirty = true;
            }
            Key::Char('m') => {
                self.shading = (self.shading + 1) % SHADING_MODES.len();
                self.dirty = true;
            }
            Key::Char('p') => {
                self.projection = match self.projection {
                    Projection::Perspective => Projection::Orthographic,
                    _ => Projection::Perspective,
                };
                self.dirty = true;
            }
            Key::Char('a') => {
                self.aa = !self.aa;
                self.dirty = true;
            }
            Key::Char('l') => self.light_mode = !self.light_mode,
            Key::Char('o') => {
                self.turntable = !self.turntable;
                self.dirty = true;
            }
            Key::Char('t') => {
                let i = TONE_MAPS.iter().position(|t| *t == self.tone_map);
                self.tone_map = TONE_MAPS[i.map_or(0, |i| (i + 1) % TONE_MAPS.len())];
                self.dirty = true;
            }
            Key::Char(']') => self.set_exposure(self.exposure * EXPOSURE_STEP),
            Key::Char('[') => self.set_exposure(self.exposure / EXPOSURE_STEP),
            Key::Char('0') => self.set_exposure(1.0),
            Key::Char('g') => {
                self.gi = !self.gi;
                self.dirty = true;
            }
            Key::Char('.' | '>') => {
                self.ambient = if self.ambient <= 0.0 {
                    0.05
                } else {
                    (self.ambient * AMBIENT_STEP).min(MAX_AMBIENT)
                };
                self.dirty = true;
            }
            Key::Char(',' | '<') => {
                let a = self.ambient / AMBIENT_STEP;
                self.ambient = if a < 0.04 { 0.0 } else { a };
                self.dirty = true;
            }
            Key::Char(' ') => {
                // Space plays / pauses the scene animation; scenes
                // without one get the turntable instead.
                match self.animation.as_mut() {
                    Some(a) => a.playing = !a.playing,
                    None => self.turntable = !self.turntable,
                }
                self.dirty = true;
            }
            Key::Char('h') | Key::F1 => {
                self.show_hud = !self.show_hud;
                self.dirty = true;
            }
            Key::Char(_) => {}
        }
        None
    }

    /// Advance turntable + animation by `dt` seconds. Returns true when
    /// the view changed (and was marked dirty).
    pub fn tick(&mut self, dt: f32) -> bool {
        if !(dt.is_finite() && dt > 0.0) {
            return false;
        }
        let mut changed = false;
        if self.turntable && self.drag.is_none() {
            self.azimuth = wrap_degrees(self.azimuth + TURNTABLE_DEG_PER_SEC * dt);
            changed = true;
        }
        if let Some(a) = self.animation.as_mut() {
            if a.playing {
                a.time += dt;
                if let Some(d) = a.duration.filter(|d| *d > 0.0) {
                    a.time %= d;
                }
                changed = true;
            }
        }
        if changed {
            self.dirty = true;
        }
        changed
    }

    /// Build the renderer options for a `width × height` frame at
    /// supersampling factor `aa` (1 = off).
    #[allow(clippy::needless_update)]
    pub fn render_options(&self, width: u32, height: u32, aa: u32) -> RenderOptions {
        RenderOptions {
            width: width.max(1),
            height: height.max(1),
            background: BackgroundColor(BACKGROUND),
            shading: self.shading_mode(),
            projection: self.projection,
            light: LightSpec {
                azimuth_deg: self.light_azimuth,
                elevation_deg: self.light_elevation,
                intensity: 1.0,
            },
            camera: Some(CameraSpec {
                elevation_deg: self.elevation,
                azimuth_deg: self.azimuth,
                distance: self.distance,
            }),
            camera_target_offset: self.pan,
            aa: aa.clamp(1, 8),
            exposure: self.exposure,
            tone_map: self.tone_map,
            ambient: self.ambient,
            path_trace: PathTraceOptions {
                max_bounces: if self.gi {
                    PathTraceOptions::default().max_bounces
                } else {
                    1
                },
                ..PathTraceOptions::default()
            },
            time: self.animation.map(|a| a.time),
            ..RenderOptions::default()
        }
    }
}

fn wrap_degrees(d: f32) -> f32 {
    let w = d.rem_euclid(360.0);
    if w > 180.0 {
        w - 360.0
    } else {
        w
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(s: &mut ViewerState, c: char) -> Option<Command> {
        s.handle(InputEvent::Key(Key::Char(c)))
    }

    fn drag(s: &mut ViewerState, button: MouseButton, from: (f64, f64), to: (f64, f64)) {
        s.handle(InputEvent::MouseDown {
            button,
            x: from.0,
            y: from.1,
        });
        s.handle(InputEvent::MouseMove { x: to.0, y: to.1 });
        s.handle(InputEvent::MouseUp { button });
    }

    #[test]
    fn left_drag_orbits_and_clamps_elevation() {
        let mut s = ViewerState::default();
        let az0 = s.azimuth;
        s.handle(InputEvent::MouseDown {
            button: MouseButton::Left,
            x: 100.0,
            y: 100.0,
        });
        assert!(s.interacting());
        s.handle(InputEvent::MouseMove { x: 150.0, y: 100.0 });
        assert!((s.azimuth - (az0 - 50.0 * DRAG_DEG_PER_PX)).abs() < 1e-4);
        // Huge vertical drag pins elevation to the clamp.
        s.handle(InputEvent::MouseMove {
            x: 150.0,
            y: 10_000.0,
        });
        assert_eq!(s.elevation, MAX_ELEVATION);
        s.handle(InputEvent::MouseMove {
            x: 150.0,
            y: -10_000.0,
        });
        assert_eq!(s.elevation, -MAX_ELEVATION);
        s.handle(InputEvent::MouseUp {
            button: MouseButton::Left,
        });
        assert!(!s.interacting());
        // Motion without a button held changes nothing.
        let before = s.clone();
        s.handle(InputEvent::MouseMove { x: 0.0, y: 0.0 });
        assert_eq!(s.azimuth, before.azimuth);
    }

    #[test]
    fn azimuth_wraps() {
        let mut s = ViewerState::default();
        for _ in 0..10 {
            drag(&mut s, MouseButton::Left, (0.0, 0.0), (400.0, 0.0));
        }
        assert!((-180.0..=180.0).contains(&s.azimuth));
    }

    #[test]
    fn wheel_and_keys_zoom_within_bounds() {
        let mut s = ViewerState::default();
        s.handle(InputEvent::Wheel(1.0));
        assert!(s.distance < 1.0);
        press(&mut s, '-');
        press(&mut s, '-');
        assert!(s.distance > 1.0);
        for _ in 0..200 {
            s.handle(InputEvent::Wheel(1.0));
        }
        assert_eq!(s.distance, MIN_DISTANCE);
        for _ in 0..200 {
            press(&mut s, '-');
        }
        assert_eq!(s.distance, MAX_DISTANCE);
        press(&mut s, '+');
        assert!(s.distance < MAX_DISTANCE);
    }

    #[test]
    fn shift_drag_and_l_move_the_light_not_the_camera() {
        let mut s = ViewerState::default();
        let (az, el, laz) = (s.azimuth, s.elevation, s.light_azimuth);
        s.handle(InputEvent::Shift(true));
        drag(&mut s, MouseButton::Left, (0.0, 0.0), (20.0, 0.0));
        assert_eq!((s.azimuth, s.elevation), (az, el));
        assert!((s.light_azimuth - (laz + 20.0 * DRAG_DEG_PER_PX)).abs() < 1e-4);
        s.handle(InputEvent::Shift(false));
        drag(&mut s, MouseButton::Left, (0.0, 0.0), (20.0, 0.0));
        assert_ne!(s.azimuth, az);

        let mut s = ViewerState::default();
        press(&mut s, 'l');
        let laz = s.light_azimuth;
        drag(&mut s, MouseButton::Left, (0.0, 0.0), (0.0, 0.0));
        drag(&mut s, MouseButton::Left, (0.0, 0.0), (-10.0, 0.0));
        assert!((s.light_azimuth - (laz - 10.0 * DRAG_DEG_PER_PX)).abs() < 1e-4);
    }

    fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
        a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
    }

    #[test]
    fn right_and_middle_drag_pan_in_the_screen_plane() {
        let mut s = ViewerState::default().with_scene_extent(2.0);
        s.handle(InputEvent::Resized(400, 300));
        let before = s.clone();
        drag(&mut s, MouseButton::Right, (0.0, 0.0), (50.0, 0.0));
        assert_eq!((s.azimuth, s.elevation), (before.azimuth, before.elevation));
        let (side, up) = s.camera_basis();
        let k = s.world_per_pixel();
        assert!((dot(s.pan, side) + 50.0 * k).abs() < 1e-4);
        assert!(dot(s.pan, up).abs() < 1e-4);
        drag(&mut s, MouseButton::Middle, (0.0, 0.0), (0.0, 30.0));
        assert!((dot(s.pan, up) - 30.0 * k).abs() < 1e-4);
        // Perpendicular to the view direction (screen-plane pan).
        let el = s.elevation.to_radians();
        let az = s.azimuth.to_radians();
        let dir = [el.cos() * az.sin(), el.sin(), el.cos() * az.cos()];
        assert!(dot(s.pan, dir).abs() < 1e-4);
        assert_eq!(s.render_options(4, 4, 1).camera_target_offset, s.pan);
        assert!(!s.interacting());
        press(&mut s, 'r');
        assert_eq!(s.pan, [0.0; 3]);
    }

    #[test]
    fn pan_scale_follows_zoom_and_extent() {
        let mut s = ViewerState::default();
        s.handle(InputEvent::Resized(400, 300));
        s.scene_extent = 1.0;
        let k1 = s.world_per_pixel();
        s.scene_extent = 3.0;
        assert!((s.world_per_pixel() - 3.0 * k1).abs() < 1e-5);
        s.handle(InputEvent::Wheel(2.0));
        assert!(s.world_per_pixel() < 3.0 * k1);
    }

    #[test]
    fn camera_basis_is_orthonormal() {
        let mut s = ViewerState::default();
        for (az, el) in [(0.0, 0.0), (35.0, 25.0), (-120.0, -80.0), (179.0, 89.0)] {
            s.azimuth = az;
            s.elevation = el;
            let (side, up) = s.camera_basis();
            assert!((dot(side, side) - 1.0).abs() < 1e-4);
            assert!((dot(up, up) - 1.0).abs() < 1e-4);
            assert!(dot(side, up).abs() < 1e-4);
            assert!(up[1] > 0.0, "up stays on the +Y side");
        }
    }

    #[test]
    fn reset_restores_defaults() {
        let mut s = ViewerState::default();
        drag(&mut s, MouseButton::Left, (0.0, 0.0), (37.0, 13.0));
        s.handle(InputEvent::Wheel(3.0));
        s.handle(InputEvent::Shift(true));
        drag(&mut s, MouseButton::Left, (0.0, 0.0), (37.0, 13.0));
        press(&mut s, 'r');
        let d = ViewerState::default();
        assert_eq!(
            (s.azimuth, s.elevation, s.distance),
            (d.azimuth, d.elevation, d.distance)
        );
        assert_eq!(
            (s.light_azimuth, s.light_elevation),
            (d.light_azimuth, d.light_elevation)
        );
        s.handle(InputEvent::Wheel(3.0));
        s.handle(InputEvent::Key(Key::Home));
        assert_eq!(s.distance, d.distance);
    }

    #[test]
    fn mode_keys() {
        let mut s = ViewerState::default();
        for (i, c) in ('1'..='7').enumerate() {
            press(&mut s, c);
            assert_eq!(s.shading_mode(), SHADING_MODES[i]);
        }
        press(&mut s, 'm');
        assert_eq!(s.shading_mode(), SHADING_MODES[0]);
        press(&mut s, 'p');
        assert_eq!(s.projection, Projection::Orthographic);
        press(&mut s, 'p');
        assert_eq!(s.projection, Projection::Perspective);
        let aa = s.aa;
        press(&mut s, 'a');
        assert_eq!(s.aa, !aa);
        assert_eq!(press(&mut s, 'b'), Some(Command::CycleBackend));
        assert_eq!(press(&mut s, 'f'), Some(Command::ToggleFullscreen));
        assert_eq!(press(&mut s, 'q'), Some(Command::Quit));
        assert_eq!(s.handle(InputEvent::Key(Key::Escape)), Some(Command::Quit));
        assert_eq!(s.handle(InputEvent::CloseRequested), Some(Command::Quit));
        let hud = s.show_hud;
        s.handle(InputEvent::Key(Key::F1));
        assert_eq!(s.show_hud, !hud);
    }

    #[test]
    fn dirty_tracking() {
        let mut s = ViewerState::default();
        assert!(s.take_dirty(), "fresh state needs a first frame");
        assert!(!s.take_dirty());
        // Pressing the button alone doesn't redraw; moving does.
        s.handle(InputEvent::MouseDown {
            button: MouseButton::Left,
            x: 0.0,
            y: 0.0,
        });
        assert!(!s.take_dirty());
        s.handle(InputEvent::MouseMove { x: 3.0, y: 0.0 });
        assert!(s.take_dirty());
        // Release triggers the full-quality refine.
        s.handle(InputEvent::MouseUp {
            button: MouseButton::Left,
        });
        assert!(s.take_dirty());
        // Idle tick without animation doesn't redraw.
        assert!(!s.tick(0.1));
        assert!(!s.take_dirty());
    }

    #[test]
    fn space_toggles_turntable_without_animation() {
        let mut s = ViewerState::default();
        assert!(!s.animating());
        press(&mut s, ' ');
        assert!(s.turntable && s.animating());
        let az = s.azimuth;
        assert!(s.tick(1.0));
        assert!((s.azimuth - wrap_degrees(az + TURNTABLE_DEG_PER_SEC)).abs() < 1e-3);
        press(&mut s, 'o');
        assert!(!s.animating());
    }

    #[test]
    fn space_plays_and_pauses_animation_and_time_loops() {
        let mut s = ViewerState::new(Some(Some(2.0)));
        assert!(s.animating());
        s.tick(1.5);
        assert_eq!(s.render_options(8, 8, 1).time, Some(1.5));
        s.tick(1.0);
        let t = s.animation.unwrap().time;
        assert!((t - 0.5).abs() < 1e-4, "loops at the duration: {t}");
        press(&mut s, ' ');
        assert!(!s.animating());
        assert!(!s.tick(1.0));
        assert!((s.animation.unwrap().time - 0.5).abs() < 1e-4);
        assert!(
            !s.turntable,
            "space drives the animation, not the turntable"
        );
    }

    #[test]
    fn exposure_and_tone_map_keys() {
        let mut s = ViewerState::default();
        assert_eq!(s.shading_mode(), ShadingMode::Pbr);
        assert_eq!(s.tone_map, ToneMap::Clamp);
        press(&mut s, 't');
        assert_eq!(s.tone_map, ToneMap::Reinhard);
        press(&mut s, 't');
        assert_eq!(s.tone_map, ToneMap::AcesFitted);
        press(&mut s, 't');
        assert_eq!(s.tone_map, ToneMap::Clamp);
        press(&mut s, ']');
        press(&mut s, ']');
        assert!((s.exposure - 2.0).abs() < 1e-4);
        for _ in 0..100 {
            press(&mut s, '[');
        }
        assert_eq!(s.exposure, MIN_EXPOSURE);
        press(&mut s, '0');
        assert_eq!(s.exposure, 1.0);
        press(&mut s, ']');
        press(&mut s, 't');
        let o = s.render_options(4, 4, 1);
        assert!(o.camera.is_some(), "ortho zoom needs Some(CameraSpec)");
        assert_eq!(o.tone_map, ToneMap::Reinhard);
        assert!((o.exposure - EXPOSURE_STEP).abs() < 1e-4);
        assert!(o.validate().is_ok());
    }

    #[test]
    fn gi_and_ambient_keys() {
        let mut s = ViewerState::default();
        let full = PathTraceOptions::default().max_bounces;
        assert_eq!(s.render_options(4, 4, 1).path_trace.max_bounces, full);
        press(&mut s, 'g');
        assert_eq!(s.render_options(4, 4, 1).path_trace.max_bounces, 1);
        press(&mut s, 'g');
        assert_eq!(s.render_options(4, 4, 1).path_trace.max_bounces, full);
        let a0 = s.ambient;
        press(&mut s, '.');
        assert!(s.ambient > a0);
        for _ in 0..40 {
            press(&mut s, '.');
        }
        assert_eq!(s.ambient, MAX_AMBIENT);
        for _ in 0..40 {
            press(&mut s, ',');
        }
        assert_eq!(s.ambient, 0.0);
        press(&mut s, '>');
        assert!(s.ambient > 0.0);
        assert_eq!(s.render_options(4, 4, 1).ambient, s.ambient);
        assert!(s.render_options(4, 4, 1).validate().is_ok());
    }

    #[test]
    fn render_options_reflect_state() {
        let mut s = ViewerState::default();
        press(&mut s, '5');
        press(&mut s, 'p');
        s.handle(InputEvent::Wheel(1.0));
        let o = s.render_options(320, 200, 3);
        assert_eq!((o.width, o.height, o.aa), (320, 200, 3));
        assert_eq!(o.shading, ShadingMode::Wireframe);
        assert_eq!(o.projection, Projection::Orthographic);
        let cam = o.camera.unwrap();
        assert_eq!(cam.azimuth_deg, s.azimuth);
        assert_eq!(cam.distance, s.distance);
        assert_eq!(o.time, None);
        assert!(o.validate().is_ok());
        // Degenerate sizes are clamped to something renderable.
        let o = s.render_options(0, 0, 99);
        assert!(o.validate().is_ok());
    }
}

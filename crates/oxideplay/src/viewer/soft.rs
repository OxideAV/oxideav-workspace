//! Software render path: CPU backends from `oxideav-render` run on a
//! worker thread so the window stays responsive while a frame renders.
//!
//! ## Progressive refinement
//!
//! Every view change starts a new *generation*. For a generation the
//! loop walks a [`Pass`] ladder from [`plan_passes`]: while the user is
//! dragging only a reduced-resolution preview is rendered; once idle
//! the ladder refines to full resolution and then to full resolution
//! with supersampling. Each finished pass is shown immediately, and a
//! newer generation abandons the rest of the ladder (stale results are
//! still shown — they're closer to the target than nothing).
//!
//! ## Path tracer
//!
//! The `pathtrace` backend is driven progressively instead of through
//! `Renderer::render`: the worker keeps one [`PathTracer`] for the
//! scene, [`PathTracer::sync`]s it with each job's options (which only
//! resets the accumulation when something affecting radiance changed —
//! not for exposure / tone map / background), and for a progressive
//! pass refines in small time-budgeted batches, publishing the running
//! estimate at an increasing cadence until it converges or a newer job
//! arrives (which preempts it between batches).

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use oxideav_mesh3d::Scene3D;
use oxideav_render::{
    PathTracer, RenderOptions, RenderRegistry, Renderer, RgbaImage, TextureResolver,
};

/// Registry name of the progressive path tracer.
pub const PATHTRACE: &str = "pathtrace";

/// Samples per pixel the idle progressive pass accumulates up to.
pub const PT_TARGET_SPP: u32 = 1024;

/// Wall-clock budget of one refine batch (preemption latency is about
/// one batch, or one sample when a single sample takes longer).
const PT_BATCH_BUDGET: Duration = Duration::from_millis(40);

/// One rung of the refinement ladder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pass {
    /// Resolution divisor (1 = full window resolution).
    pub div: u32,
    /// Supersampling factor handed to the renderer.
    pub aa: u32,
    /// Keep refining (path tracer) until converged or preempted,
    /// publishing intermediate images.
    pub progressive: bool,
}

impl Pass {
    pub const fn new(div: u32, aa: u32) -> Self {
        Self {
            div,
            aa,
            progressive: false,
        }
    }
}

/// Every software backend the linked `oxideav-render` exposes, by
/// registry name, cheapest first (`scanline`, `raycast`, then the rest —
/// `pathtrace` — in registry order).
pub fn software_backends() -> Vec<String> {
    let reg = registry();
    let mut names: Vec<String> = reg.names().into_iter().map(str::to_string).collect();
    names.sort_by_key(|n| (n != "scanline", n != "raycast"));
    names
}

fn registry() -> RenderRegistry {
    let mut reg = RenderRegistry::new();
    oxideav_meta::populate_render_registry(&mut reg);
    reg
}

/// Pick the refinement ladder for one generation.
///
/// * `interacting` (dragging, or animating): preview only, at
///   `preview_div` (adapted by the caller from measured frame times);
///   the path tracer takes one sample per pixel.
/// * otherwise: full resolution, then full resolution with `aa_level`
///   supersampling when AA is on. Ray-traced backends get a cheap
///   reduced-resolution rung first because their full frame is slow.
///   The path tracer's full-resolution rung is progressive (its
///   jittered samples already anti-alias, so there's no AA rung).
pub fn plan_passes(backend: &str, interacting: bool, preview_div: u32, aa_level: u32) -> Vec<Pass> {
    let preview_div = preview_div.clamp(1, 8);
    if interacting {
        return vec![Pass::new(preview_div, 1)];
    }
    let mut out = Vec::new();
    if backend != "scanline" && preview_div > 1 {
        out.push(Pass::new(preview_div, 1));
    }
    if backend == PATHTRACE {
        out.push(Pass {
            progressive: true,
            ..Pass::new(1, 1)
        });
        return out;
    }
    out.push(Pass::new(1, 1));
    if aa_level > 1 {
        out.push(Pass::new(1, aa_level));
    }
    out
}

/// True when a newer generation may preempt the job in flight instead
/// of waiting for it (progressive passes run until converged).
pub fn preemptible(backend: &str) -> bool {
    backend == PATHTRACE
}

/// Adapt the preview divisor so an interactive preview frame lands
/// around 15–45 ms.
pub fn adapt_preview_div(div: u32, preview_time: Duration) -> u32 {
    let ms = preview_time.as_secs_f64() * 1000.0;
    if ms > 45.0 {
        (div + 1).min(8)
    } else if ms < 15.0 {
        div.saturating_sub(1).max(1)
    } else {
        div
    }
}

/// How often a progressive pass publishes its running estimate: fast
/// while the image is still noisy, slower as it converges (each publish
/// costs a full-frame resolve).
pub fn publish_interval(samples: u32) -> Duration {
    match samples {
        0..=7 => Duration::from_millis(80),
        8..=63 => Duration::from_millis(200),
        _ => Duration::from_millis(500),
    }
}

/// One persistent path tracer driven in time-budgeted batches.
pub struct PtSession {
    tracer: PathTracer,
}

impl PtSession {
    pub fn new(resolver: Arc<dyn TextureResolver>) -> Self {
        Self {
            tracer: PathTracer::with_texture_resolver(resolver),
        }
    }

    /// Bring the tracer in line with `opts` (`target` samples per
    /// pixel). Returns true when the accumulation was reset.
    pub fn sync(&mut self, scene: &Scene3D, opts: &RenderOptions, target: u32) -> bool {
        let mut o = opts.clone();
        o.path_trace.samples_per_pixel = target.max(1);
        self.tracer.sync(scene, &o)
    }

    /// Refine for about `budget` (at least one sample unless already
    /// converged). Returns the samples per pixel accumulated so far.
    pub fn refine_for(&mut self, budget: Duration) -> u32 {
        let start = Instant::now();
        while !self.tracer.is_converged() {
            self.tracer.refine(1);
            if start.elapsed() >= budget {
                break;
            }
        }
        self.tracer.samples()
    }

    pub fn samples(&self) -> u32 {
        self.tracer.samples()
    }

    pub fn is_converged(&self) -> bool {
        self.tracer.is_converged()
    }

    pub fn image(&self) -> RgbaImage {
        self.tracer.image()
    }
}

struct Job {
    generation: u64,
    pass: Pass,
    backend: String,
    opts: RenderOptions,
}

/// Path-tracer progress attached to a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Progress {
    /// Samples per pixel accumulated.
    pub samples: u32,
    /// Samples per pixel the pass is heading for.
    pub target: u32,
    /// The accumulation was reset by this job's sync.
    pub reset: bool,
}

/// One software frame (a finished pass, or an intermediate progressive
/// estimate).
pub struct SoftFrame {
    pub generation: u64,
    pub pass: Pass,
    pub backend: String,
    pub elapsed: Duration,
    pub image: Result<RgbaImage, String>,
    pub progress: Option<Progress>,
}

enum WorkerMsg {
    Frame(Box<SoftFrame>),
    /// A job finished (or was skipped / preempted). Exactly one per
    /// submitted job.
    Done,
}

/// Handle on the render worker thread.
pub struct SoftWorker {
    tx: Option<Sender<Job>>,
    rx: Receiver<WorkerMsg>,
    handle: Option<JoinHandle<()>>,
    in_flight: usize,
}

impl SoftWorker {
    /// Start the worker. `resolver` is installed into every backend
    /// through `Renderer::set_texture_resolver` (and into the path
    /// tracer).
    pub fn spawn(scene: Arc<Scene3D>, resolver: Arc<dyn TextureResolver>) -> Self {
        let (job_tx, job_rx) = mpsc::channel::<Job>();
        let (msg_tx, msg_rx) = mpsc::channel::<WorkerMsg>();
        let handle = std::thread::Builder::new()
            .name("oxideplay-render".into())
            .spawn(move || worker_main(scene, resolver, job_rx, msg_tx))
            .expect("spawn render worker");
        Self {
            tx: Some(job_tx),
            rx: msg_rx,
            handle: Some(handle),
            in_flight: 0,
        }
    }

    /// True while a submitted job hasn't completed yet.
    pub fn busy(&self) -> bool {
        self.in_flight > 0
    }

    /// Queue one pass. Normally one job is in flight at a time (the
    /// caller checks [`Self::busy`]); a progressive job is preempted
    /// by submitting the next one while it runs.
    pub fn submit(&mut self, generation: u64, pass: Pass, backend: &str, opts: RenderOptions) {
        if let Some(tx) = self.tx.as_ref() {
            if tx
                .send(Job {
                    generation,
                    pass,
                    backend: backend.to_string(),
                    opts,
                })
                .is_ok()
            {
                self.in_flight += 1;
            }
        }
    }

    fn take(&mut self, msg: WorkerMsg) -> Option<SoftFrame> {
        match msg {
            WorkerMsg::Frame(f) => Some(*f),
            WorkerMsg::Done => {
                self.in_flight = self.in_flight.saturating_sub(1);
                None
            }
        }
    }

    /// Non-blocking poll for the next frame.
    pub fn try_recv(&mut self) -> Option<SoftFrame> {
        loop {
            match self.rx.try_recv() {
                Ok(m) => {
                    if let Some(f) = self.take(m) {
                        return Some(f);
                    }
                }
                Err(TryRecvError::Empty) => return None,
                Err(TryRecvError::Disconnected) => {
                    self.in_flight = 0;
                    return None;
                }
            }
        }
    }

    /// Blocking poll with a timeout (headless tests).
    #[cfg(test)]
    pub fn recv_timeout(&mut self, timeout: Duration) -> Option<SoftFrame> {
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.checked_duration_since(Instant::now())?;
            let m = self.rx.recv_timeout(left).ok()?;
            if let Some(f) = self.take(m) {
                return Some(f);
            }
        }
    }
}

impl Drop for SoftWorker {
    fn drop(&mut self) {
        // Closing the job channel ends the worker loop (a progressive
        // pass notices between batches). Don't join: a slow ray-traced
        // frame may still be running, and the process is about to exit
        // anyway.
        self.tx.take();
        drop(self.handle.take());
    }
}

/// Build a renderer by registry name with the texture decoder
/// installed (backends that don't sample textures ignore it).
fn make_backend(
    reg: &RenderRegistry,
    name: &str,
    resolver: &Arc<dyn TextureResolver>,
) -> Result<Box<dyn Renderer>, String> {
    let mut r = reg.make(name).map_err(|e| e.to_string())?;
    r.set_texture_resolver(resolver.clone());
    Ok(r)
}

/// Why a progressive job stopped.
enum Stop {
    Finished,
    Preempted(Box<Job>),
    Disconnected,
}

fn worker_main(
    scene: Arc<Scene3D>,
    resolver: Arc<dyn TextureResolver>,
    jobs: Receiver<Job>,
    out: Sender<WorkerMsg>,
) {
    let reg = registry();
    let mut renderers: HashMap<String, Box<dyn Renderer>> = HashMap::new();
    let mut pt = PtSession::new(resolver.clone());
    let mut next: Option<Job> = None;
    loop {
        let mut job = match next.take() {
            Some(j) => j,
            None => match jobs.recv() {
                Ok(j) => j,
                Err(_) => return,
            },
        };
        // Skip to the newest queued job — anything older is stale.
        while let Ok(newer) = jobs.try_recv() {
            if out.send(WorkerMsg::Done).is_err() {
                return;
            }
            job = newer;
        }
        let start = Instant::now();
        let frame = |job: &Job, image, progress| {
            WorkerMsg::Frame(Box::new(SoftFrame {
                generation: job.generation,
                pass: job.pass,
                backend: job.backend.clone(),
                elapsed: start.elapsed(),
                image,
                progress,
            }))
        };

        if job.backend == PATHTRACE {
            let target = if job.pass.progressive {
                PT_TARGET_SPP
            } else {
                1
            };
            let reset = pt.sync(&scene, &job.opts, target);
            if !job.pass.progressive {
                let samples = pt.refine_for(Duration::ZERO);
                let progress = Some(Progress {
                    samples,
                    target,
                    reset,
                });
                if out.send(frame(&job, Ok(pt.image()), progress)).is_err()
                    || out.send(WorkerMsg::Done).is_err()
                {
                    return;
                }
                continue;
            }
            let stop = run_progressive(&mut pt, &jobs, &out, &job, reset, start);
            if out.send(WorkerMsg::Done).is_err() {
                return;
            }
            match stop {
                Stop::Finished => {}
                Stop::Preempted(j) => next = Some(*j),
                Stop::Disconnected => return,
            }
            continue;
        }

        if !renderers.contains_key(&job.backend) {
            match make_backend(&reg, &job.backend, &resolver) {
                Ok(r) => {
                    renderers.insert(job.backend.clone(), r);
                }
                Err(e) => {
                    if out.send(frame(&job, Err(e), None)).is_err()
                        || out.send(WorkerMsg::Done).is_err()
                    {
                        return;
                    }
                    continue;
                }
            }
        }
        let image = match renderers.get_mut(&job.backend) {
            Some(r) => r.render(&scene, &job.opts).map_err(|e| e.to_string()),
            None => Err(format!("backend '{}' unavailable", job.backend)),
        };
        if out.send(frame(&job, image, None)).is_err() || out.send(WorkerMsg::Done).is_err() {
            return;
        }
    }
}

/// Refine `pt` in budgeted batches, publishing the running estimate,
/// until converged or a newer job arrives.
fn run_progressive(
    pt: &mut PtSession,
    jobs: &Receiver<Job>,
    out: &Sender<WorkerMsg>,
    job: &Job,
    mut reset: bool,
    start: Instant,
) -> Stop {
    // Publish right after the first batch.
    let mut last_publish: Option<Instant> = None;
    loop {
        if !pt.is_converged() {
            pt.refine_for(PT_BATCH_BUDGET);
        }
        let samples = pt.samples();
        let converged = pt.is_converged();
        let due = last_publish.is_none()
            || last_publish.is_some_and(|t| t.elapsed() >= publish_interval(samples));
        if due || converged {
            let msg = WorkerMsg::Frame(Box::new(SoftFrame {
                generation: job.generation,
                pass: job.pass,
                backend: job.backend.clone(),
                elapsed: start.elapsed(),
                image: Ok(pt.image()),
                progress: Some(Progress {
                    samples,
                    target: PT_TARGET_SPP,
                    reset,
                }),
            }));
            if out.send(msg).is_err() {
                return Stop::Disconnected;
            }
            reset = false;
            last_publish = Some(Instant::now());
        }
        if converged {
            return Stop::Finished;
        }
        match jobs.try_recv() {
            Ok(j) => return Stop::Preempted(Box::new(j)),
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => return Stop::Disconnected,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ladder_shapes() {
        assert_eq!(plan_passes("scanline", true, 3, 4), vec![Pass::new(3, 1)]);
        assert_eq!(
            plan_passes("scanline", false, 3, 4),
            vec![Pass::new(1, 1), Pass::new(1, 4)]
        );
        assert_eq!(plan_passes("scanline", false, 3, 1), vec![Pass::new(1, 1)]);
        assert_eq!(
            plan_passes("raycast", false, 2, 4),
            vec![Pass::new(2, 1), Pass::new(1, 1), Pass::new(1, 4)]
        );
        assert_eq!(plan_passes(PATHTRACE, true, 3, 4), vec![Pass::new(3, 1)]);
        assert_eq!(
            plan_passes(PATHTRACE, false, 3, 4),
            vec![
                Pass::new(3, 1),
                Pass {
                    progressive: true,
                    ..Pass::new(1, 1)
                }
            ]
        );
        assert!(preemptible(PATHTRACE) && !preemptible("scanline"));
    }

    #[test]
    fn preview_div_adapts() {
        assert_eq!(adapt_preview_div(2, Duration::from_millis(100)), 3);
        assert_eq!(adapt_preview_div(8, Duration::from_millis(100)), 8);
        assert_eq!(adapt_preview_div(2, Duration::from_millis(5)), 1);
        assert_eq!(adapt_preview_div(1, Duration::from_millis(5)), 1);
        assert_eq!(adapt_preview_div(2, Duration::from_millis(30)), 2);
    }

    #[test]
    fn publish_cadence_slows_as_samples_grow() {
        assert!(publish_interval(0) < publish_interval(16));
        assert!(publish_interval(16) < publish_interval(512));
    }

    fn cube() -> Arc<Scene3D> {
        Arc::new(crate::viewer::tests_support::cube_scene())
    }

    fn opts(state: &crate::viewer::state::ViewerState) -> RenderOptions {
        state.render_options(40, 30, 1)
    }

    /// The session accumulates, keeps samples across display-only
    /// changes (exposure, tone map) and resets on a camera change.
    #[test]
    fn path_trace_session_accumulates_and_resets_on_camera_only() {
        use crate::viewer::state::{InputEvent, Key, MouseButton, ViewerState};
        let scene = cube();
        let mut pt = PtSession::new(crate::viewer::texture_resolver("cube.stl"));
        let mut st = ViewerState::default();
        assert!(pt.sync(&scene, &opts(&st), 64), "first sync prepares");
        let s1 = pt.refine_for(Duration::ZERO);
        let s2 = pt.refine_for(Duration::ZERO);
        assert!(s1 >= 1 && s2 > s1, "samples grow: {s1} → {s2}");
        assert!(!pt.sync(&scene, &opts(&st), 64), "unchanged options");
        st.handle(InputEvent::Key(Key::Char(']')));
        st.handle(InputEvent::Key(Key::Char('t')));
        assert!(!pt.sync(&scene, &opts(&st), 64), "exposure / tone map");
        assert_eq!(pt.samples(), s2, "display-only change keeps samples");
        let bright = pt.image();
        assert_eq!((bright.width, bright.height), (40, 30));
        // Raising the target alone doesn't reset either.
        assert!(!pt.sync(&scene, &opts(&st), PT_TARGET_SPP));
        // Orbit drag → camera moved → reset.
        st.handle(InputEvent::MouseDown {
            button: MouseButton::Left,
            x: 0.0,
            y: 0.0,
        });
        st.handle(InputEvent::MouseMove { x: 10.0, y: 0.0 });
        assert!(pt.sync(&scene, &opts(&st), 64), "camera change resets");
        assert_eq!(pt.samples(), 0);
        // GI toggle changes radiance → reset.
        pt.refine_for(Duration::ZERO);
        st.handle(InputEvent::Key(Key::Char('g')));
        assert!(pt.sync(&scene, &opts(&st), 64), "gi change resets");
        // Converges at the target.
        assert!(!pt.sync(&scene, &opts(&st), 3));
        while !pt.is_converged() {
            pt.refine_for(Duration::from_millis(5));
        }
        assert_eq!(pt.samples(), 3);
    }

    /// Next frame of `generation` from the worker (skipping older ones).
    fn frame_of(w: &mut SoftWorker, generation: u64) -> SoftFrame {
        loop {
            let f = w
                .recv_timeout(Duration::from_secs(60))
                .expect("worker answered");
            if f.generation == generation {
                return f;
            }
        }
    }

    /// The worker's progressive pass publishes increasing sample
    /// counts, is preempted by a newer job, keeps accumulating across
    /// an exposure change and restarts on a camera change.
    #[test]
    fn progressive_worker_loop() {
        use crate::viewer::state::{InputEvent, Key, MouseButton, ViewerState};
        let mut w = SoftWorker::spawn(cube(), crate::viewer::texture_resolver("cube.stl"));
        let mut st = ViewerState::default();
        let prog = Pass {
            progressive: true,
            ..Pass::new(1, 1)
        };
        w.submit(1, prog, PATHTRACE, opts(&st));
        let mut last = 0;
        for i in 0..3 {
            let f = frame_of(&mut w, 1);
            let p = f.progress.expect("progress");
            assert_eq!(p.target, PT_TARGET_SPP);
            assert_eq!(p.reset, i == 0, "only the first frame reports the reset");
            assert!(p.samples > last, "samples grow: {last} → {}", p.samples);
            assert!(f.image.is_ok());
            last = p.samples;
        }
        assert!(w.busy(), "still refining toward {PT_TARGET_SPP}");

        // Exposure change preempts but keeps the accumulation.
        st.handle(InputEvent::Key(Key::Char(']')));
        w.submit(2, prog, PATHTRACE, opts(&st));
        let f = frame_of(&mut w, 2);
        let p = f.progress.unwrap();
        assert!(!p.reset, "exposure must not reset");
        assert!(
            p.samples > last,
            "kept accumulating: {last} → {}",
            p.samples
        );
        let before_move = p.samples;

        // Camera change restarts from scratch.
        st.handle(InputEvent::MouseDown {
            button: MouseButton::Left,
            x: 0.0,
            y: 0.0,
        });
        st.handle(InputEvent::MouseMove { x: 25.0, y: 5.0 });
        w.submit(3, prog, PATHTRACE, opts(&st));
        let f = frame_of(&mut w, 3);
        let p = f.progress.unwrap();
        assert!(p.reset, "camera change resets");
        assert!(p.samples < before_move, "restarted: {} spp", p.samples);

        // A non-progressive preview pass is one sample and completes.
        w.submit(4, Pass::new(2, 1), PATHTRACE, st.render_options(20, 15, 1));
        let f = frame_of(&mut w, 4);
        assert_eq!(f.progress.unwrap().samples, 1);
        assert_eq!(f.image.unwrap().width, 20);
        let deadline = Instant::now() + Duration::from_secs(10);
        while w.busy() && Instant::now() < deadline {
            let _ = w.recv_timeout(Duration::from_millis(50));
        }
        assert!(!w.busy(), "every job reported done");
    }

    #[test]
    fn backends_are_ordered_and_include_the_path_tracer() {
        let names = software_backends();
        assert_eq!(names.first().map(String::as_str), Some("scanline"));
        assert!(names.iter().any(|n| n == PATHTRACE), "{names:?}");
    }
}

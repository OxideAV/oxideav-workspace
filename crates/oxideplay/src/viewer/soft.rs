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
//! A progressive backend (a future path tracer) slots in by returning a
//! longer ladder for its name — e.g. increasing sample counts — without
//! the loop changing: the loop only knows "submit the next pass when
//! the worker is free, show what comes back".

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use oxideav_mesh3d::Scene3D;
use oxideav_render::{
    RenderOptions, RenderRegistry, Renderer, RgbaImage, ScanlineRenderer, TextureResolver,
};

/// One rung of the refinement ladder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pass {
    /// Resolution divisor (1 = full window resolution).
    pub div: u32,
    /// Supersampling factor handed to the renderer.
    pub aa: u32,
}

/// Every software backend the linked `oxideav-render` exposes, by
/// registry name, cheapest first (`scanline` leads; anything else —
/// `raycast`, a future `pathtrace` — follows in registry order).
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
/// * `interacting`: a drag is in progress — preview only, at
///   `preview_div` (adapted by the caller from measured frame times).
/// * otherwise: full resolution, then full resolution with `aa_level`
///   supersampling when AA is on. Ray-traced backends get a cheap
///   half-resolution rung first because their full frame is slow.
pub fn plan_passes(backend: &str, interacting: bool, preview_div: u32, aa_level: u32) -> Vec<Pass> {
    let preview_div = preview_div.clamp(1, 8);
    if interacting {
        return vec![Pass {
            div: preview_div,
            aa: 1,
        }];
    }
    let mut out = Vec::new();
    if backend != "scanline" && preview_div > 1 {
        out.push(Pass {
            div: preview_div,
            aa: 1,
        });
    }
    out.push(Pass { div: 1, aa: 1 });
    if aa_level > 1 {
        out.push(Pass {
            div: 1,
            aa: aa_level,
        });
    }
    out
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

struct Job {
    generation: u64,
    pass: Pass,
    backend: String,
    opts: RenderOptions,
}

/// One finished software frame.
pub struct SoftFrame {
    pub generation: u64,
    pub pass: Pass,
    pub backend: String,
    pub elapsed: Duration,
    pub image: Result<RgbaImage, String>,
}

/// Handle on the render worker thread.
pub struct SoftWorker {
    tx: Option<Sender<Job>>,
    rx: Receiver<SoftFrame>,
    handle: Option<JoinHandle<()>>,
    in_flight: bool,
}

impl SoftWorker {
    /// Start the worker. `resolver` decodes textures for the backends
    /// that take one (today: `scanline`).
    pub fn spawn(scene: Arc<Scene3D>, resolver: Arc<dyn TextureResolver>) -> Self {
        let (job_tx, job_rx) = mpsc::channel::<Job>();
        let (frame_tx, frame_rx) = mpsc::channel::<SoftFrame>();
        let handle = std::thread::Builder::new()
            .name("oxideplay-render".into())
            .spawn(move || worker_main(scene, resolver, job_rx, frame_tx))
            .expect("spawn render worker");
        Self {
            tx: Some(job_tx),
            rx: frame_rx,
            handle: Some(handle),
            in_flight: false,
        }
    }

    /// True while a submitted pass hasn't come back yet.
    pub fn busy(&self) -> bool {
        self.in_flight
    }

    /// Queue one pass. Only one pass is in flight at a time — the
    /// caller checks [`Self::busy`] first.
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
                self.in_flight = true;
            }
        }
    }

    /// Non-blocking poll for a finished frame.
    pub fn try_recv(&mut self) -> Option<SoftFrame> {
        match self.rx.try_recv() {
            Ok(f) => {
                self.in_flight = false;
                Some(f)
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.in_flight = false;
                None
            }
        }
    }

    /// Blocking poll with a timeout (headless tests).
    #[cfg(test)]
    pub fn recv_timeout(&mut self, timeout: Duration) -> Option<SoftFrame> {
        let f = self.rx.recv_timeout(timeout).ok()?;
        self.in_flight = false;
        Some(f)
    }
}

impl Drop for SoftWorker {
    fn drop(&mut self) {
        // Closing the job channel ends the worker loop. Don't join: a
        // slow ray-traced frame may still be running, and the process
        // is about to exit anyway.
        self.tx.take();
        drop(self.handle.take());
    }
}

/// Build a renderer by registry name, wiring the texture decoder into
/// backends that expose a hook for it.
fn make_backend(
    reg: &RenderRegistry,
    name: &str,
    resolver: &Arc<dyn TextureResolver>,
) -> Result<Box<dyn Renderer>, String> {
    if name == "scanline" {
        return Ok(Box::new(ScanlineRenderer::with_texture_resolver(
            resolver.clone(),
        )));
    }
    reg.make(name).map_err(|e| e.to_string())
}

fn worker_main(
    scene: Arc<Scene3D>,
    resolver: Arc<dyn TextureResolver>,
    jobs: Receiver<Job>,
    frames: Sender<SoftFrame>,
) {
    let reg = registry();
    let mut renderers: HashMap<String, Box<dyn Renderer>> = HashMap::new();
    while let Ok(mut job) = jobs.recv() {
        // Skip to the newest queued job — anything older is stale.
        while let Ok(newer) = jobs.try_recv() {
            job = newer;
        }
        let start = Instant::now();
        if !renderers.contains_key(&job.backend) {
            match make_backend(&reg, &job.backend, &resolver) {
                Ok(r) => {
                    renderers.insert(job.backend.clone(), r);
                }
                Err(e) => {
                    let failed = SoftFrame {
                        generation: job.generation,
                        pass: job.pass,
                        backend: job.backend,
                        elapsed: start.elapsed(),
                        image: Err(e),
                    };
                    if frames.send(failed).is_err() {
                        break;
                    }
                    continue;
                }
            }
        }
        let image = match renderers.get_mut(&job.backend) {
            Some(r) => r.render(&scene, &job.opts).map_err(|e| e.to_string()),
            None => Err(format!("backend '{}' unavailable", job.backend)),
        };
        let frame = SoftFrame {
            generation: job.generation,
            pass: job.pass,
            backend: job.backend,
            elapsed: start.elapsed(),
            image,
        };
        if frames.send(frame).is_err() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ladder_shapes() {
        assert_eq!(
            plan_passes("scanline", true, 3, 4),
            vec![Pass { div: 3, aa: 1 }]
        );
        assert_eq!(
            plan_passes("scanline", false, 3, 4),
            vec![Pass { div: 1, aa: 1 }, Pass { div: 1, aa: 4 }]
        );
        assert_eq!(
            plan_passes("scanline", false, 3, 1),
            vec![Pass { div: 1, aa: 1 }]
        );
        assert_eq!(
            plan_passes("raycast", false, 2, 4),
            vec![
                Pass { div: 2, aa: 1 },
                Pass { div: 1, aa: 1 },
                Pass { div: 1, aa: 4 }
            ]
        );
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
    fn scanline_is_listed_first() {
        let names = software_backends();
        assert_eq!(names.first().map(String::as_str), Some("scanline"));
    }
}

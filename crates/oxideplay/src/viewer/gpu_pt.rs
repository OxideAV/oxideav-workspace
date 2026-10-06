//! GPU path tracer (`oxideav-render-vulkan`'s `GpuPathTracer`) on the
//! viewer's own wgpu device.
//!
//! The tracer is created lazily — the first creation in a process
//! compiles the path-tracing kernel (~1.5 s) — on a background thread,
//! so the window keeps pumping and the HUD can say "compiling
//! shaders…". Each frame then runs [`step`]: `sync` (resets only on
//! radiance changes), a non-blocking `refine` of a few samples sized by
//! [`adapt_spp`] from the measured frame interval, and `draw_texture`
//! (no readback) — which the frontend blits like any other frame.

use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::Duration;

use oxideav_mesh3d::Scene3D;
use oxideav_render::{RenderOptions, TextureResolver};
use oxideav_render_vulkan::GpuPathTracer;

use super::soft::{Progress, PT_TARGET_SPP};

/// Samples per frame are kept within this range.
pub const MIN_SPP: u32 = 1;
pub const MAX_SPP: u32 = 256;

/// Adapt the samples refined per frame from the wall-clock interval
/// between successive refining frames. With FIFO presentation the
/// interval is about `max(vsync, GPU time)`, so growing while frames
/// stay near the refresh interval and shrinking once they run long
/// settles the GPU work near ~10–15 ms per frame without a blocking
/// `finish()`.
pub fn adapt_spp(spp: u32, interval: Duration) -> u32 {
    let ms = interval.as_secs_f64() * 1000.0;
    let next = if ms < 20.0 {
        spp + spp.div_ceil(4)
    } else if ms > 30.0 {
        spp * 2 / 3
    } else {
        spp
    };
    next.clamp(MIN_SPP, MAX_SPP)
}

/// Lifecycle of the lazily created tracer.
pub enum Slot {
    /// Not requested yet.
    Idle,
    /// Kernel compiling on a background thread.
    Compiling(Receiver<Result<GpuPathTracer, String>>),
    Ready(Box<GpuPathTracer>),
    Failed(String),
}

pub use super::GpuPtStatus as Status;

impl Slot {
    /// Start creating the tracer on `device` if not done yet, and
    /// report its state.
    pub fn poll(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        info: &wgpu::AdapterInfo,
        resolver: Option<&Arc<dyn TextureResolver>>,
    ) -> Status {
        if let Slot::Idle = self {
            let (tx, rx) = mpsc::channel();
            let (d, q, i) = (device.clone(), queue.clone(), info.clone());
            let r = resolver.cloned();
            let spawned = std::thread::Builder::new()
                .name("oxideplay-gpu-pt-compile".into())
                .spawn(move || {
                    let res = GpuPathTracer::from_device(d, q, i)
                        .map(|mut pt| {
                            if let Some(r) = r {
                                pt.set_texture_resolver(r);
                            }
                            pt
                        })
                        .map_err(|e| e.to_string());
                    let _ = tx.send(res);
                });
            *self = match spawned {
                Ok(_) => Slot::Compiling(rx),
                Err(e) => Slot::Failed(format!("spawn: {e}")),
            };
        }
        if let Slot::Compiling(rx) = self {
            match rx.try_recv() {
                Ok(Ok(pt)) => *self = Slot::Ready(Box::new(pt)),
                Ok(Err(e)) => *self = Slot::Failed(e),
                Err(TryRecvError::Empty) => return Status::Compiling,
                Err(TryRecvError::Disconnected) => {
                    *self = Slot::Failed("compile thread died".into())
                }
            }
        }
        match self {
            Slot::Ready(_) => Status::Ready,
            Slot::Failed(e) => Status::Failed(e.clone()),
            _ => Status::Compiling,
        }
    }

    pub fn tracer(&mut self) -> Option<&mut GpuPathTracer> {
        match self {
            Slot::Ready(pt) => Some(pt),
            _ => None,
        }
    }
}

/// One progressive frame: sync with `opts` (heading for
/// [`PT_TARGET_SPP`]), queue up to `spp` samples unless converged, and
/// resolve into the tracer's output texture.
pub fn step<'a>(
    pt: &'a mut GpuPathTracer,
    scene: &Scene3D,
    opts: &RenderOptions,
    spp: u32,
) -> oxideav_render::Result<(Progress, &'a wgpu::Texture)> {
    let mut o = opts.clone();
    o.path_trace.samples_per_pixel = PT_TARGET_SPP;
    let reset = pt.sync(scene, &o)?;
    if !pt.is_converged() {
        let left = PT_TARGET_SPP.saturating_sub(pt.samples());
        pt.refine(spp.clamp(MIN_SPP, MAX_SPP).min(left))?;
    }
    let progress = Progress {
        samples: pt.samples(),
        target: PT_TARGET_SPP,
        reset,
    };
    let tex = pt.draw_texture()?;
    Ok((progress, tex))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::viewer::state::{InputEvent, Key, MouseButton, ViewerState, BACKGROUND};
    use std::time::Instant;

    #[test]
    fn spp_adapts_to_frame_interval() {
        assert!(adapt_spp(4, Duration::from_millis(16)) > 4);
        assert!(adapt_spp(12, Duration::from_millis(50)) < 12);
        assert_eq!(adapt_spp(8, Duration::from_millis(25)), 8);
        assert_eq!(adapt_spp(1, Duration::from_millis(500)), MIN_SPP);
        assert_eq!(adapt_spp(MAX_SPP, Duration::from_millis(1)), MAX_SPP);
    }

    fn read_rgba(device: &wgpu::Device, queue: &wgpu::Queue, tex: &wgpu::Texture) -> Vec<u8> {
        let (w, h) = (tex.width(), tex.height());
        let row = (w * 4).div_ceil(256) * 256;
        let buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: (row * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            tex.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(h),
                },
            },
            tex.size(),
        );
        queue.submit(Some(enc.finish()));
        let slice = buf.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("poll");
        let data = slice.get_mapped_range();
        (0..h as usize)
            .flat_map(|y| data[y * row as usize..y * row as usize + w as usize * 4].to_vec())
            .collect()
    }

    /// Lazy creation through [`Slot`] on a shared device, then the
    /// per-frame [`step`]: samples grow, the texture shows the model,
    /// exposure keeps the accumulation, a camera drag restarts it.
    #[test]
    fn gpu_path_tracer_refines_progressively() {
        let Ok(raster) = oxideav_render_vulkan::GpuRenderer::new() else {
            eprintln!("no GPU adapter — skipping");
            return;
        };
        let (device, queue) = (raster.device().clone(), raster.queue().clone());
        let info = raster.adapter_info().clone();
        let mut slot = Slot::Idle;
        let resolver = crate::viewer::texture_resolver("cube.stl");
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            match slot.poll(&device, &queue, &info, Some(&resolver)) {
                Status::Ready => break,
                Status::Compiling => {
                    assert!(Instant::now() < deadline, "kernel compile timed out");
                    std::thread::sleep(Duration::from_millis(20));
                }
                Status::Failed(e) => {
                    eprintln!("GPU path tracer unavailable ({e}) — skipping");
                    return;
                }
            }
        }
        let pt = slot.tracer().expect("ready");
        let scene = crate::viewer::tests_support::cube_scene();
        let mut st = ViewerState::default();
        let (w, h) = (48, 36);

        let (p1, _) = step(pt, &scene, &st.render_options(w, h, 1), 4).unwrap();
        assert!(p1.reset && p1.samples == 4, "{p1:?}");
        let (p2, tex) = step(pt, &scene, &st.render_options(w, h, 1), 4).unwrap();
        assert!(!p2.reset && p2.samples == 8, "{p2:?}");
        assert_eq!((tex.width(), tex.height()), (w, h));
        assert_eq!(tex.format(), oxideav_render_vulkan::COLOR_FORMAT);
        let px = read_rgba(&device, &queue, tex);
        let covered = px.chunks(4).filter(|p| p[..3] != BACKGROUND[..3]).count();
        assert!(covered > (w * h / 20) as usize, "model covers {covered} px");

        // Exposure: display-only, keeps accumulating.
        st.handle(InputEvent::Key(Key::Char(']')));
        let (p3, _) = step(pt, &scene, &st.render_options(w, h, 1), 4).unwrap();
        assert!(!p3.reset && p3.samples == 12, "{p3:?}");
        // Camera drag: restart at full resolution.
        st.handle(InputEvent::MouseDown {
            button: MouseButton::Left,
            x: 0.0,
            y: 0.0,
        });
        st.handle(InputEvent::MouseMove { x: 15.0, y: 0.0 });
        let (p4, tex) = step(pt, &scene, &st.render_options(w, h, 1), 2).unwrap();
        assert!(p4.reset && p4.samples == 2, "{p4:?}");
        assert_eq!(tex.width(), w);
        pt.finish().unwrap();
    }

    /// Headless frontend backed by a real GPU: raster + lazily created
    /// path tracer, scripted by what the HUD shows.
    struct HeadlessGpu {
        raster: oxideav_render_vulkan::GpuRenderer,
        scene: Option<oxideav_render_vulkan::GpuScene>,
        info: wgpu::AdapterInfo,
        slot: Slot,
        stage: u32,
        mark: usize,
        deadline: Instant,
        huds: Vec<crate::viewer::Hud>,
    }

    fn row<'a>(h: &'a crate::viewer::Hud, k: &str) -> Option<&'a str> {
        h.rows.iter().find(|(l, _)| l == k).map(|(_, v)| v.as_str())
    }

    fn spp(h: &crate::viewer::Hud) -> Option<u32> {
        row(h, "samples")?.split_whitespace().next()?.parse().ok()
    }

    impl crate::viewer::Frontend for HeadlessGpu {
        fn pump(&mut self, timeout: Duration) -> Vec<InputEvent> {
            std::thread::sleep(timeout.min(Duration::from_millis(5)));
            assert!(
                Instant::now() < self.deadline,
                "stalled at stage {}",
                self.stage
            );
            let since = &self.huds[self.mark..];
            let converged = since
                .iter()
                .any(|h| row(h, "samples").is_some_and(|v| v.contains("converged")));
            let go = |stage: &mut u32, mark: &mut usize, n: usize| {
                *stage += 1;
                *mark = n;
            };
            let n = self.huds.len();
            match self.stage {
                // Raster → GPU path trace.
                0 => {
                    go(&mut self.stage, &mut self.mark, n);
                    vec![InputEvent::Key(Key::Char('b'))]
                }
                1 if converged => {
                    go(&mut self.stage, &mut self.mark, n);
                    vec![InputEvent::Key(Key::Char(']'))]
                }
                // Exposure re-resolves without resetting.
                2 if !since.is_empty() => {
                    go(&mut self.stage, &mut self.mark, n);
                    vec![
                        InputEvent::MouseDown {
                            button: MouseButton::Left,
                            x: 0.0,
                            y: 0.0,
                        },
                        InputEvent::MouseMove { x: 12.0, y: 0.0 },
                        InputEvent::MouseUp {
                            button: MouseButton::Left,
                        },
                    ]
                }
                3 if converged => {
                    go(&mut self.stage, &mut self.mark, n);
                    vec![InputEvent::Key(Key::Char('q'))]
                }
                _ => Vec::new(),
            }
        }
        fn size(&self) -> (u32, u32) {
            (64, 48)
        }
        fn set_image(&mut self, _img: &oxideav_render::RgbaImage) -> oxideav_core::Result<()> {
            Ok(())
        }
        fn present(&mut self, hud: &crate::viewer::Hud) -> oxideav_core::Result<()> {
            self.huds.push(hud.clone());
            Ok(())
        }
        fn toggle_fullscreen(&mut self) {}
        fn describe(&self) -> String {
            "headless gpu".into()
        }
        fn gpu_adapter(&self) -> Option<String> {
            Some(self.raster.adapter_summary())
        }
        fn gpu_upload(&mut self, scene: &Scene3D, opts: &RenderOptions) -> Option<usize> {
            let gs = self.raster.upload(scene, opts);
            let n = gs.triangle_count();
            self.scene = Some(gs);
            Some(n)
        }
        fn gpu_draw(&mut self, opts: &RenderOptions) -> oxideav_core::Result<()> {
            let gs = self.scene.as_mut().unwrap();
            self.raster
                .draw(gs, opts)
                .map(|_| ())
                .map_err(|e| oxideav_core::Error::other(e.to_string()))
        }
        fn gpu_pt_poll(&mut self) -> Option<Status> {
            let (d, q) = (self.raster.device().clone(), self.raster.queue().clone());
            Some(self.slot.poll(&d, &q, &self.info, None))
        }
        fn gpu_pt_frame(
            &mut self,
            scene: &Scene3D,
            opts: &RenderOptions,
            spp: u32,
        ) -> oxideav_core::Result<Progress> {
            let pt = self.slot.tracer().unwrap();
            step(pt, scene, opts, spp)
                .map(|(p, _)| p)
                .map_err(|e| oxideav_core::Error::other(e.to_string()))
        }
    }

    /// The real viewer loop on the GPU path tracer: lazy compile shown
    /// in the HUD, progressive refinement to convergence (then idle),
    /// exposure without reset, drag restart, reconvergence.
    #[test]
    fn viewer_loop_drives_the_gpu_path_tracer() {
        let Ok(raster) = oxideav_render_vulkan::GpuRenderer::new() else {
            eprintln!("no GPU adapter — skipping");
            return;
        };
        let info = raster.adapter_info().clone();
        // Not every backend compiles the path-tracer kernel (D3D12's
        // FXC rejects it); the viewer then reports it unavailable, which
        // this loop test cannot drive — skip like its sibling test.
        if let Err(e) = oxideav_render_vulkan::GpuPathTracer::from_device(
            raster.device().clone(),
            raster.queue().clone(),
            info.clone(),
        ) {
            eprintln!("GPU path tracer unavailable ({e}) — skipping");
            return;
        }
        let mut fe = HeadlessGpu {
            raster,
            scene: None,
            info,
            slot: Slot::Idle,
            stage: 0,
            mark: 0,
            deadline: Instant::now() + Duration::from_secs(120),
            huds: Vec::new(),
        };
        let scene = crate::viewer::tests_support::cube_scene();
        crate::viewer::run_loop(
            &mut fe,
            scene,
            "cube.stl",
            crate::viewer::texture_resolver("cube.stl"),
        )
        .unwrap();
        assert_eq!(fe.stage, 4);
        let pt: Vec<&crate::viewer::Hud> = fe
            .huds
            .iter()
            .filter(|h| h.summary.contains("gpu-pathtrace"))
            .collect();
        assert!(
            pt.iter()
                .any(|h| row(h, "status") == Some("compiling shaders…")),
            "compile status shown"
        );
        let counts: Vec<u32> = pt.iter().filter_map(|h| spp(h)).collect();
        let full = counts.iter().position(|c| *c == PT_TARGET_SPP).unwrap();
        // The exposure step keeps the converged accumulation; the drag
        // then restarts it below the target before reconverging.
        let restart = full
            + counts[full..]
                .iter()
                .position(|c| *c < PT_TARGET_SPP)
                .expect("drag restarted the accumulation");
        assert!(
            counts[full..restart].iter().all(|c| *c == PT_TARGET_SPP),
            "{counts:?}"
        );
        assert_eq!(*counts.last().unwrap(), PT_TARGET_SPP, "{counts:?}");
        // Samples per frame adapted upward from the start value.
        assert!(
            counts.windows(2).any(|w| w[1] > w[0] + 2),
            "spp per frame grew: {counts:?}"
        );
    }
}

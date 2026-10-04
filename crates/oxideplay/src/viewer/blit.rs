//! Fullscreen-triangle blit of the viewer's current frame (a UNORM
//! texture holding sRGB-encoded RGBA8, from the software path or from
//! `oxideav-render-vulkan`) onto a render target. The target must be a
//! UNORM view — for an `*Srgb` surface the caller renders through a
//! `remove_srgb_suffix()` view format — so bytes pass through without
//! a second sRGB encode.

use super::state::BACKGROUND;

pub struct Blit {
    pipeline: wgpu::RenderPipeline,
    bgl: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

impl Blit {
    /// Build the pipeline for render targets of `target_format`.
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("viewer-blit"),
            source: wgpu::ShaderSource::Wgsl(include_str!("blit.wgsl").into()),
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("viewer-blit-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("viewer-blit-pl"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("viewer-blit"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        // Linear filtering upscales reduced-resolution previews; at 1:1
        // it samples texel centres exactly (identity).
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("viewer-blit"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        Self {
            pipeline,
            bgl,
            sampler,
        }
    }

    /// Bind group sampling `source`.
    pub fn bind_group(&self, device: &wgpu::Device, source: &wgpu::TextureView) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("viewer-blit-bg"),
            layout: &self.bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(source),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        })
    }

    /// Clear `target` to the viewer background and, when `source` is
    /// given, stretch it over the whole target.
    pub fn encode(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        source: Option<&wgpu::BindGroup>,
    ) {
        let bg = BACKGROUND;
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("viewer-blit"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    // The clear goes through a UNORM view, so these are
                    // the sRGB-encoded bytes verbatim.
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: bg[0] as f64 / 255.0,
                        g: bg[1] as f64 / 255.0,
                        b: bg[2] as f64 / 255.0,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        if let Some(bg) = source {
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, bg, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}

#[cfg(all(test, feature = "viewer-gpu"))]
mod tests {
    use super::*;
    use crate::viewer::state::ViewerState;
    use oxideav_render_vulkan::{GpuRenderer, COLOR_FORMAT};

    fn read_rgba(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        tex: &wgpu::Texture,
        w: u32,
        h: u32,
    ) -> Vec<u8> {
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
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        queue.submit(Some(enc.finish()));
        let slice = buf.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("poll");
        let data = slice.get_mapped_range();
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h as usize {
            let o = y * row as usize;
            out.extend_from_slice(&data[o..o + w as usize * 4]);
        }
        out
    }

    /// GPU path end to end without a window: draw the viewer's options
    /// on the GPU, blit onto a UNORM target and onto an sRGB target
    /// through a UNORM view (the `*Srgb` surface case), and check both
    /// reproduce the renderer's sRGB-encoded bytes exactly — i.e. no
    /// double encode.
    #[test]
    fn gpu_frame_blits_without_double_srgb_encode() {
        let Ok(mut gpu) = GpuRenderer::new() else {
            eprintln!("no GPU adapter — skipping");
            return;
        };
        let scene = crate::viewer::tests_support::cube_scene();
        let state = ViewerState::default();
        let (w, h) = (64, 48);
        let opts = state.render_options(w, h, 2);
        let mut gs = gpu.upload(&scene, &opts);
        assert_eq!(gs.triangle_count(), 12);
        let device = gpu.device().clone();
        let queue = gpu.queue().clone();
        let tex = gpu.draw(&mut gs, &opts).expect("gpu draw");
        assert_eq!(tex.format(), COLOR_FORMAT);
        assert_eq!((tex.width(), tex.height()), (w, h));
        let source_view = tex.create_view(&Default::default());
        let reference = read_rgba(&device, &queue, tex, w, h);
        let bg = BACKGROUND;
        let covered = reference.chunks(4).filter(|p| p[..3] != bg[..3]).count();
        assert!(covered > (w * h / 20) as usize, "cube covers {covered} px");

        for surface_format in [
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureFormat::Rgba8UnormSrgb,
        ] {
            let target_format = surface_format.remove_srgb_suffix();
            let target = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("fake-surface"),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: surface_format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[target_format],
            });
            let view = target.create_view(&wgpu::TextureViewDescriptor {
                format: Some(target_format),
                ..Default::default()
            });
            let blit = Blit::new(&device, target_format);
            let bgp = blit.bind_group(&device, &source_view);
            let mut enc = device.create_command_encoder(&Default::default());
            blit.encode(&mut enc, &view, Some(&bgp));
            queue.submit(Some(enc.finish()));
            let got = read_rgba(&device, &queue, &target, w, h);
            let max_diff = got
                .chunks(4)
                .zip(reference.chunks(4))
                .flat_map(|(a, b)| (0..3).map(move |i| a[i].abs_diff(b[i])))
                .max()
                .unwrap_or(0);
            assert!(
                max_diff <= 1,
                "{surface_format:?}: blit changed colours by {max_diff}"
            );
        }
    }

    /// The GPU backend honours the viewer's pan 1:1 too (orbit target
    /// offset, perspective and ortho).
    #[test]
    fn gpu_pan_is_one_to_one() {
        use crate::viewer::state::{InputEvent, Key, MouseButton};
        use oxideav_render::Renderer;
        let Ok(mut gpu) = GpuRenderer::new() else {
            eprintln!("no GPU adapter — skipping");
            return;
        };
        let scene = crate::viewer::tests_support::quad_scene();
        let (w, h) = (160u32, 120u32);
        let centroid = |img: &oxideav_render::RgbaImage| {
            let (mut sx, mut n) = (0.0_f64, 0u32);
            for y in 0..h {
                for x in 0..w {
                    if img.pixel(x, y).is_some_and(|p| p[..3] != BACKGROUND[..3]) {
                        sx += x as f64;
                        n += 1;
                    }
                }
            }
            sx / n.max(1) as f64
        };
        for ortho in [false, true] {
            let mut s =
                ViewerState::default().with_scene_extent(crate::viewer::scene_extent(&scene));
            s.handle(InputEvent::Resized(w, h));
            s.azimuth = 0.0;
            s.elevation = 0.0;
            s.handle(InputEvent::Key(Key::Char('3')));
            if ortho {
                s.handle(InputEvent::Key(Key::Char('p')));
            }
            let x0 = centroid(&gpu.render(&scene, &s.render_options(w, h, 1)).unwrap());
            s.handle(InputEvent::MouseDown {
                button: MouseButton::Middle,
                x: 0.0,
                y: 0.0,
            });
            s.handle(InputEvent::MouseMove { x: -15.0, y: 0.0 });
            s.handle(InputEvent::MouseUp {
                button: MouseButton::Middle,
            });
            let x1 = centroid(&gpu.render(&scene, &s.render_options(w, h, 1)).unwrap());
            assert!(
                (x1 - x0 + 15.0).abs() < 2.0,
                "ortho={ortho}: -15 px drag moved the model {:.2} px",
                x1 - x0
            );
        }
    }
}

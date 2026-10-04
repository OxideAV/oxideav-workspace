// Fullscreen-triangle blit of the viewer's current frame onto the
// surface. The source texture holds sRGB-encoded values in a UNORM
// format and the render target is a UNORM view of the surface, so the
// values pass straight through with no encode / decode.

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs(@builtin(vertex_index) i: u32) -> VsOut {
    // (0,0) (2,0) (0,2) in uv → one triangle covering the viewport.
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: VsOut;
    out.pos = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    out.uv = uv;
    return out;
}

@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
    let c = textureSample(src, samp, in.uv);
    return vec4<f32>(c.rgb, 1.0);
}

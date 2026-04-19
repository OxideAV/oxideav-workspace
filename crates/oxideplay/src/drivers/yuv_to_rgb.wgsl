// YUV 4:2:0 planar → RGB, BT.709 full-range.
// Chroma U/V textures are half resolution in each axis; `textureSample`
// with linear filtering handles the upsampling.

@group(0) @binding(0) var y_tex: texture_2d<f32>;
@group(0) @binding(1) var u_tex: texture_2d<f32>;
@group(0) @binding(2) var v_tex: texture_2d<f32>;
@group(0) @binding(3) var samp:  sampler;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0)       uv:  vec2<f32>,
}

// Full-screen triangle — three vertices covering the clip-space
// square. Vertex index 0 → (-1,+1), 1 → (+3,+1), 2 → (-1,-3).
// UVs map (0,0)..(2,2); the fragments outside (0..1) are clipped.
@vertex
fn vs(@builtin(vertex_index) i: u32) -> VsOut {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    var o: VsOut;
    o.pos = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    o.uv  = vec2<f32>(x, y);
    return o;
}

@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
    let y = textureSample(y_tex, samp, in.uv).r;
    let u = textureSample(u_tex, samp, in.uv).r - 0.5;
    let v = textureSample(v_tex, samp, in.uv).r - 0.5;
    // BT.709 full-range: R = Y + 1.5748 V, G = Y - 0.1873 U - 0.4681 V,
    //                   B = Y + 1.8556 U
    let r = y + 1.5748 * v;
    let g = y - 0.1873 * u - 0.4681 * v;
    let b = y + 1.8556 * u;
    return vec4<f32>(r, g, b, 1.0);
}

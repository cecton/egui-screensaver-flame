// Original shader by @kuvkar (https://www.shadertoy.com/view/4tXXRn),
// via Blake Bowen's "Flame in the wind" CodePen
// (https://codepen.io/osublake/pen/pqNXoq). Ported to WGSL almost verbatim:
// the PixiJS filter's `vTextureCoord`/`filterArea` plumbing is replaced by a
// plain `uv` varying, since for a fullscreen draw
// `vTextureCoord * filterArea.xy / dimensions` reduces to just the incoming
// UV.

struct Params {
    time : f32,
    scale : f32,
    dimensions : vec2<f32>,
};

@group(0) @binding(0) var<uniform> params : Params;
@group(0) @binding(1) var noise_tex : texture_2d<f32>;
@group(0) @binding(2) var noise_samp : sampler;

struct VertexOutput {
    @builtin(position) position : vec4<f32>,
    @location(0) uv : vec2<f32>,
};

// A single triangle covering the whole clip-space square and beyond
// (vertices at (-1,-1), (3,-1), (-1,3)); the parts outside [-1,1] are
// clipped away, leaving a fullscreen quad with no extra draw call.
@vertex
fn vs_main(@builtin(vertex_index) index : u32) -> VertexOutput {
    var corners = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    let pos = corners[index];
    var out : VertexOutput;
    // Top-left-origin (Y-down) UV, the same convention the reference
    // shader's `vTextureCoord` used: `fs_main` does its own
    // `uv.y = 1.0 - uv.y` first to convert *to* Y-up for the rest of its
    // math.
    out.uv = vec2<f32>(pos.x * 0.5 + 0.5, 0.5 - pos.y * 0.5);
    out.position = vec4<f32>(pos, 0.0, 1.0);
    return out;
}

fn rand(co : vec2<f32>) -> f32 {
    return fract(sin(dot(co, vec2<f32>(12.9898, 78.233))) * 43758.5453);
}

// GLSL's `smoothstep` tolerates `edge0 > edge1` (the shader relies on it);
// WGSL's does not, so spell the formula out.
fn smooth_step(edge0 : f32, edge1 : f32, x : f32) -> f32 {
    let t = clamp((x - edge0) / (edge1 - edge0), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

fn rotz(angle : f32) -> mat2x2<f32> {
    return mat2x2<f32>(
        vec2<f32>(cos(angle), -sin(angle)),
        vec2<f32>(sin(angle), cos(angle)),
    );
}

fn noise(uv : vec2<f32>) -> f32 {
    return textureSampleLevel(noise_tex, noise_samp, uv, 0.0).r;
}

// Fractal Brownian motion
fn fbm(uv : vec2<f32>) -> f32 {
    var n = (noise(uv) - 0.5) * 0.5;
    n += (noise(uv * 2.0) - 0.5) * 0.5 * 0.5;
    n += (noise(uv * 3.0) - 0.5) * 0.5 * 0.5 * 0.5;
    return n + 0.5;
}

@fragment
fn fs_main(in : VertexOutput) -> @location(0) vec4<f32> {
    let time = params.time;
    var uv = in.uv;
    uv.y = 1.0 - uv.y;

    let _uv = uv;
    uv -= vec2<f32>(0.5);
    uv.y /= params.dimensions.x / params.dimensions.y;

    let centerUV = uv;

    // height variation from fbm
    let variationH = fbm(vec2<f32>(time * 0.3)) * 1.1;

    // flame "speed"
    let offset = vec2<f32>(0.0, -time * 0.05);

    // flame turbulence — computed in the original, unscaled coordinate
    // space so `scale` doesn't shear the sway/turbulence pattern (a
    // rotation matrix applied to an anisotropically-scaled vector would
    // skew it).
    let f = fbm(uv * 0.1 + offset);
    let l = max(0.1, length(uv));
    uv += rotz(((f - 0.5) / l) * smooth_step(-0.2, 0.4, _uv.y) * 0.45) * uv;

    // `scale` widens the flame horizontally only, applied after the
    // turbulence/rotation above (so that math stays undistorted) and left
    // out of `centerUV`/the halo below, so the vertical taper, turbulence
    // pattern, and background halo all stay the same size — only the
    // flame's horizontal thickness grows.
    uv.x /= params.scale;

    // flame thickness
    var flame = 1.3 - abs(uv.x) * 5.0;

    // bottom of flame. `pow` of a negative base is undefined in WGSL; the
    // result is clamped to [0, 1] below anyway, so negatives become 0.
    var blueflame = pow(max(flame * 0.9, 0.0), 15.0);
    blueflame *= smooth_step(0.2, -1.0, _uv.y);
    blueflame /= max(abs(uv.x * 2.0), 1e-6);
    blueflame = clamp(blueflame, 0.0, 1.0);

    // flame
    flame *= smooth_step(1.0, variationH * 0.5, _uv.y);
    flame = clamp(flame, 0.0, 1.0);
    flame = pow(flame, 3.0);
    flame /= smooth_step(1.1, -0.1, _uv.y);

    // colors
    var col = mix(vec4<f32>(1.0, 1.0, 0.0, 0.0), vec4<f32>(1.0, 1.0, 0.6, 0.0), flame);
    col = mix(vec4<f32>(1.0, 0.0, 0.0, 0.0), col, smooth_step(0.0, 1.6, flame));
    var color = col;

    // a bit blueness on the bottom
    let bluecolor = mix(vec4<f32>(0.0, 0.0, 1.0, 0.0), color, 0.95);
    color = mix(color, bluecolor, blueflame);

    // clear bg outside of the flame
    color *= flame;
    color.a = flame;

    // bg halo
    let haloSize = 0.5;
    let centerL = 1.0 - (length(centerUV + vec2<f32>(0.0, 0.1)) / haloSize);
    let halo = vec4<f32>(0.8, 0.3, 0.3, 0.0) * 1.0 * fbm(vec2<f32>(time * 0.035)) * centerL + 0.02;
    color = mix(halo, color, color.a);

    // just a hint of noise
    color *= mix(rand(uv) + rand(uv * 0.45), 1.0, 0.9);
    return clamp(color, vec4<f32>(0.0), vec4<f32>(1.0));
}

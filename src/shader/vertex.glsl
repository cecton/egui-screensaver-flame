#if NEW_SHADER_INTERFACE
    in vec2 a_pos;
    out vec2 v_uv;
#else
    attribute vec2 a_pos;
    varying vec2 v_uv;
#endif

// A single triangle covering the whole clip-space square and beyond
// (vertices at (-1,-1), (3,-1), (-1,3)); the parts outside [-1,1] are
// clipped away, leaving a fullscreen quad with no extra draw call.
void main() {
    v_uv = a_pos * 0.5 + 0.5;
    // Flip to the same top-left-origin (Y-down) convention the reference
    // shader's `vTextureCoord` used — it does its own `uv.y = 1.0 - uv.y`
    // as the very first line of `main()` to convert *to* Y-up for the rest
    // of its math. Feeding it an already-Y-up UV here would double-flip
    // and render the flame upside down.
    v_uv.y = 1.0 - v_uv.y;
    gl_Position = vec4(a_pos, 0.0, 1.0);
}

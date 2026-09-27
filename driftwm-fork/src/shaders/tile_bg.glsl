//_DEFINES
precision highp float;
varying vec2 v_coords;
uniform sampler2D tex;
uniform float alpha;

uniform vec2 u_camera;
uniform vec2 u_tile_size;
uniform vec2 u_output_size;

void main() {
    vec2 screen_pixel = v_coords * u_output_size;
#ifdef MIRROR
    // Reduce by the 2× mirror period, not 1×. Reducing by 1× drops the camera
    // by a half-period each image-width of pan/zoom, flipping the reflection so
    // the mirror jumps instead of scrolling rigidly.
    vec2 canvas_pos = screen_pixel + mod(u_camera, 2.0 * u_tile_size);
    vec2 centered = canvas_pos + floor(u_tile_size * 0.5);
    // Triangle wave folds the [0,2) period back into [0,1] texel space: every
    // other copy is reflected, so non-seamless edges meet their mirror.
    vec2 uv = 1.0 - abs(1.0 - mod(centered / u_tile_size, 2.0));
#else
    vec2 canvas_pos = screen_pixel + mod(u_camera, u_tile_size);
    // floor(u_tile_size*0.5) centers the image on canvas (0,0) with the wrap
    // seam at ±dims/2. `floor` (not a bare 0.5×) so odd dimensions land on the
    // same integer offset as the chunked path's `image_position = -(dims/2)`,
    // whose i32 division truncates — keeping the shader fallback plane pixel-
    // aligned with the per-tile chunks drawn over it.
    vec2 centered = canvas_pos + floor(u_tile_size * 0.5);
    vec2 uv = mod(centered, u_tile_size) / u_tile_size;
#endif
    vec4 color = texture2D(tex, uv);
    #ifdef NO_ALPHA
    color = vec4(color.rgb, 1.0);
    #endif
    gl_FragColor = color * alpha;
}

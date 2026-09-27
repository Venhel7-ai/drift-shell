// Ripple — tiles an image across the canvas and animates a gentle ripple over
// it, so even a still picture drifts like a reflection on water. This is the
// thing plain `tile`/`wallpaper` can't do: a procedural effect *on* your image.
//
//     [background]
//     type = "shader"
//     path = "~/.config/driftwm/wallpapers/ripple.glsl"
//     texture = "~/Pictures/your_image.png"
//
// `tex` is the configured image. u_texture_size / u_output_size are the image's
// and the viewport's pixel sizes — texture shaders get no built-in `size`, and
// GLSL ES 1.0 has no textureSize(). Zoom is applied externally, so we work in
// canvas space. Backgrounds are opaque, so no `alpha` uniform is needed.
precision highp float;

varying vec2 v_coords;
uniform sampler2D tex;

uniform vec2 u_camera;
uniform vec2 u_output_size;
uniform vec2 u_texture_size;
uniform float u_time;

// --- Tweak these ---
const float WAVES = 4.0;       // wave cycles across one image tile (integer = seamless)
const float AMPLITUDE = 12.0;  // ripple strength, in image pixels
const float SPEED = 0.8;       // animation speed
// -------------------

const float TAU = 6.2831853;

void main() {
    // Screen pixel -> canvas, in image-tile units. mod(u_camera, …) bounds the
    // value so the trig stays precise anywhere on the infinite canvas; an
    // integer WAVES keeps the ripple seamless across tile and pan-wrap edges.
    vec2 tile_uv =
        (v_coords * u_output_size + mod(u_camera, u_texture_size)) / u_texture_size;

    float t = u_time * SPEED;
    vec2 ripple = vec2(
        sin(tile_uv.y * TAU * WAVES + t),
        cos(tile_uv.x * TAU * WAVES + t * 1.3)
    ) * (AMPLITUDE / u_texture_size);

    vec3 col = texture2D(tex, fract(tile_uv + ripple)).rgb;

    // Shimmer along the wave crests.
    float crest = sin(tile_uv.y * TAU * WAVES + t) * 0.5 + 0.5;
    col += crest * crest * 0.09;

    gl_FragColor = vec4(col, 1.0);
}

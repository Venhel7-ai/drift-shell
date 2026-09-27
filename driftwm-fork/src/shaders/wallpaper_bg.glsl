//_DEFINES
precision highp float;
varying vec2 v_coords;
uniform sampler2D tex;
uniform float alpha;

void main() {
    vec4 color = texture2D(tex, v_coords);
    #ifdef NO_ALPHA
    color = vec4(color.rgb, 1.0);
    #endif
    gl_FragColor = color * alpha;
}

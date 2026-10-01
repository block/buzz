#include <flutter/runtime_effect.glsl>

uniform vec2 uSize;
uniform float uPhase;
uniform vec4 uColor;
uniform sampler2D uWordmark;
out vec4 fragColor;

float hash(vec2 p, float seed) {
  return fract(sin(dot(p, vec2(127.1, 311.7)) + seed * 17.0) * 43758.5453);
}

float noise(vec2 p, float seed) {
  vec2 cell = floor(p);
  vec2 f = fract(p);
  f = f * f * (3.0 - 2.0 * f);
  return mix(mix(hash(cell, seed), hash(cell + vec2(1, 0), seed), f.x),
             mix(hash(cell + vec2(0, 1), seed), hash(cell + vec2(1, 1), seed), f.x), f.y);
}

float fractalNoise(vec2 p, float seed) {
  float value = 0.0;
  float weight = 0.5333333;
  for (int i = 0; i < 4; i++) {
    value += weight * noise(p, seed);
    p *= 2.0;
    weight *= 0.5;
  }
  return value;
}

float seedAt(float step) {
  if (step < 1.0) return 7.0;
  if (step < 2.0) return 19.0;
  if (step < 3.0) return 11.0;
  if (step < 4.0) return 25.0;
  return 7.0;
}

// A small Gaussian kernel broadens the fuzzy edge at phone size without
// adding a separate blur pass or softening the surrounding onboarding UI.
float softAlpha(vec2 uv) {
  vec2 d = vec2(9.0) / vec2(777.0, 326.0);
  float alpha = texture(uWordmark, uv).a * 0.25;
  alpha += (texture(uWordmark, uv + vec2(d.x, 0.0)).a
          + texture(uWordmark, uv - vec2(d.x, 0.0)).a
          + texture(uWordmark, uv + vec2(0.0, d.y)).a
          + texture(uWordmark, uv - vec2(0.0, d.y)).a) * 0.125;
  alpha += (texture(uWordmark, uv + d).a
          + texture(uWordmark, uv - d).a
          + texture(uWordmark, uv + vec2(d.x, -d.y)).a
          + texture(uWordmark, uv + vec2(-d.x, d.y)).a) * 0.0625;
  return alpha;
}

void main() {
  vec2 uv = FlutterFragCoord().xy / uSize;
  vec2 p = uv * vec2(777.0, 326.0) * 0.68;
  float phase = uPhase * 4.0;
  float a = seedAt(floor(phase));
  float b = seedAt(floor(phase) + 1.0);
  float blend = smoothstep(0.0, 1.0, fract(phase));
  vec2 textureNoise = mix(
    vec2(fractalNoise(p, a), fractalNoise(p + 43.7, a)),
    vec2(fractalNoise(p, b), fractalNoise(p + 43.7, b)), blend);
  vec2 displaced = uv + (textureNoise - 0.5) * 14.4 / vec2(777.0, 326.0);
  float alpha = softAlpha(clamp(displaced, 0.0, 1.0));
  float grain = clamp((textureNoise.x + textureNoise.y) * 0.99, 0.0, 1.0);
  alpha = (alpha + alpha * grain * (1.0 - alpha)) * uColor.a;
  fragColor = vec4(uColor.rgb * alpha, alpha);
}

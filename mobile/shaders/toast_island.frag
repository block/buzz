#include <flutter/runtime_effect.glsl>

uniform vec2 uIslandCenter;
uniform vec2 uIslandHalfSize;
uniform vec2 uToastCenter;
uniform vec2 uToastHalfSize;
uniform float uBlend;
uniform float uPixel;
out vec4 fragColor;

float capsule(vec2 p, vec2 center, vec2 halfSize) {
  float radius = min(halfSize.x, halfSize.y);
  vec2 q = abs(p - center) - halfSize + radius;
  return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - radius;
}

void main() {
  vec2 p = FlutterFragCoord().xy;
  float a = capsule(p, uIslandCenter, uIslandHalfSize);
  float b = capsule(p, uToastCenter, uToastHalfSize);
  // Smooth union: both surfaces deform into a single contour near contact.
  // Its finite reach leaves the resting toast and hardware island separate.
  float distance = b;
  if (uBlend > 0.001) {
    float h = max(uBlend - abs(a - b), 0.0) / uBlend;
    distance = min(a, b) - h * h * uBlend * 0.25;
  }
  // At rest only paint the toast. The real island supplies its own exact
  // outline; our approximate scanner frame must not enlarge it.
  float alpha = 1.0 - smoothstep(-uPixel, uPixel, distance);
  fragColor = vec4(0.0, 0.0, 0.0, alpha);
}

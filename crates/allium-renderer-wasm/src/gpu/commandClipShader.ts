/** GLSL of the command clip test the semantic fragment stages and the SDF
 * glyph stage share. `insideClip` keeps a canvas point that lies in the clip
 * quad `v_clip01`, `v_clip23`. A point on a side is kept only when the clip
 * lies below or to the right of that side, so the axis-aligned rectangle a
 * clip reduces to keeps `[min, max)` on both axes, as
 * `pixel_sampling::span_contains` of the shared core does (a contract test
 * pins it). */
export const COMMAND_CLIP_GLSL = `float cross2(vec2 a, vec2 b) { return a.x * b.y - a.y * b.x; }
// Whether the clip side from start to end keeps point, for the winding that
// turns the clip's corners clockwise on the canvas.
bool clipSideKeeps(vec2 start, vec2 end, vec2 point, float winding) {
  vec2 side = (end - start) * winding;
  float turn = cross2(side, point - start);
  return turn > 0.0 || (turn == 0.0 && (side.y < 0.0 || (side.y == 0.0 && side.x > 0.0)));
}
bool insideClip(vec2 point) {
  highp vec2 p[4];
  p[0] = v_clip01.xy;
  p[1] = v_clip01.zw;
  p[2] = v_clip23.xy;
  p[3] = v_clip23.zw;
  float winding = cross2(p[1] - p[0], p[3] - p[0]) < 0.0 ? -1.0 : 1.0;
  return clipSideKeeps(p[0], p[1], point, winding)
    && clipSideKeeps(p[1], p[2], point, winding)
    && clipSideKeeps(p[2], p[3], point, winding)
    && clipSideKeeps(p[3], p[0], point, winding);
}`;

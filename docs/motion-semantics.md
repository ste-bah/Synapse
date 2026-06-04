# Motion Semantics

Issue: #648

Use separate names for timing and spatial shape:

- `velocity_profile` controls point-to-point timing. It does not describe the
  geometric path. Examples: `natural`, `instant`, `linear`, `ease_in_out`.
- `path` controls spatial shape. It belongs to `act_stroke` and supports line,
  arc, circle, cubic Bezier, polyline, and Catmull-Rom paths.
- `motion_model` controls how `act_stroke` turns the path into emitted points.
  It defaults to `{"kind":"path"}`. `{"kind":"wind_mouse",...}` is opt-in for
  seeded gravity/wind/damping point-to-point line strokes with variable step
  lengths.

`act_stroke` owns the public motion surface. Use `path` for an explicit spatial
shape, or use `target`/`to` with an optional `from` for point-to-point motion.
Leave `button` unset to move/aim; set `button` to drag along the resolved line or
path.

Migration:

- Former `act_aim` move-to-point calls map to `act_stroke` with `target` or
  `to` and no `button`.
- Former `act_drag` calls map to `act_stroke` with `from`, `to`, and `button`.
- The old `curve` field is not part of the public motion schema. Use
  `velocity_profile` for timing and `path.kind = "cubic_bezier"` for spatial
  Bezier movement.

WindMouse:

- Use `act_stroke` with `path.kind = "line"` and
  `motion_model.kind = "wind_mouse"`.
- Required positive finite parameters are `gravity`, `wind`, `max_step`, and
  `damped_distance`; `seed` is optional and makes the generated point stream
  deterministic.
- Non-line paths reject WindMouse with `TOOL_PARAMS_INVALID`; use the default
  `path` motion model for arcs, circles, cubic Beziers, polylines, and
  Catmull-Rom paths.

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

`act_aim` and `act_drag` are point-to-point tools. Their style or
`velocity_profile` changes how quickly the pointer progresses between endpoints;
callers that need a curved, closed, or multi-waypoint spatial path should use
`act_stroke`.

Migration:

- New `act_drag` calls should send `velocity_profile`.
- The old `act_drag.curve` field remains a compatibility alias for `natural`,
  `instant`, `linear`, and `ease_in_out`.
- The old `act_drag.curve = "bezier"` value is rejected with an explicit
  parameter error. Use `act_stroke.path.kind = "cubic_bezier"` for spatial
  Bezier movement, or `velocity_profile = "ease_in_out"` for point-to-point
  timing.

WindMouse:

- Use `act_stroke` with `path.kind = "line"` and
  `motion_model.kind = "wind_mouse"`.
- Required positive finite parameters are `gravity`, `wind`, `max_step`, and
  `damped_distance`; `seed` is optional and makes the generated point stream
  deterministic.
- Non-line paths reject WindMouse with `TOOL_PARAMS_INVALID`; use the default
  `path` motion model for arcs, circles, cubic Beziers, polylines, and
  Catmull-Rom paths.

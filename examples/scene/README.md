# scene

A camera, a cube turned a little, and the matrices between them: where
each corner of the cube lands on a screen of 640 by 360, and the cube's
edges drawn in text, as a GPU would draw them in pixels.

```sh
wip run examples/scene/main.wip
wip test examples/scene/main.wip
```

What it shows:

- **`std::math`:** `Vec3`, `Quat`, `Mat4` and the rest, with operators —
  `projection * view * model`, `clip.xyz() / clip.w`.
- **The conventions a GPU uses:** a matrix multiplies a vector on its
  right, so `a * b` does `b` first; the camera looks down `-z` with `y` up;
  `perspective` gives OpenGL's depth from `-1` to `1`.
- **From clip coordinates to pixels:** divided by `w`, then from `-1`–`1`
  to the screen, whose `y` counts down from the top.
- **Tests of a picture:** the centre in the middle, what is nearer
  larger, and up drawn up.

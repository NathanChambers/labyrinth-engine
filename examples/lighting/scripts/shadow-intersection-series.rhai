capture.configure(1280, 720, "shadow-intersection-series", false);
capture.set_camera([0.0, 3.0, 5.0], [0.0, 0.8, 0.0]);
capture.set_render_effects(true, false, false);
capture.set_shadow_quality(2);
for frame in 0..12 {
    capture.wait(0.5);
    capture.screenshot("frame_" + frame);
}
capture.exit();

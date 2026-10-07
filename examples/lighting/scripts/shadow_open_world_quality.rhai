// Compare directional shadow quality from a steep top-down view over the origin cube.
capture.configure(1280, 720, "shadow-open-world-quality", false);
capture.set_camera([0.0, 10.0, 0.5], [0.0, 0.0, 0.0]);
capture.set_render_effects(true, false, false);
capture.wait(0.25);
capture.set_shadow_quality(0);
capture.wait(0.1);
capture.screenshot("low");
capture.set_shadow_quality(1);
capture.wait(0.1);
capture.screenshot("medium");
capture.set_shadow_quality(2);
capture.wait(0.1);
capture.screenshot("high");
capture.exit();

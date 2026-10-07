capture.configure(1280, 720, "shadow-open-world-default", false);
capture.set_camera([0.0, 8.0, 22.0], [0.0, 7.0, 17.0]);
capture.set_render_effects(true, true, false);
capture.set_shadow_quality(2);
capture.wait(1.0);
capture.screenshot("startup_view");
capture.exit();

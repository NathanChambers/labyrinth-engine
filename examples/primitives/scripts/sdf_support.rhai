capture.configure(1280, 720, "primitives-sdf-support", false);
capture.set_render_effects(false, true, false);
capture.set_camera([4.0, 3.0, 6.0], [0.0, 0.6, 0.0]);
capture.set_render_mode("sdf_support");
capture.wait(3.0);
capture.screenshot("overview");
capture.exit();

capture.configure(1280, 720, "shadow-intersection-gi", false);
capture.set_camera([0.0, 3.0, 5.0], [0.0, 0.8, 0.0]);
capture.set_render_effects(true, true, false);
capture.set_shadow_quality(2);
capture.wait(1.0);
capture.screenshot("intersecting_objects_gi");
capture.exit();

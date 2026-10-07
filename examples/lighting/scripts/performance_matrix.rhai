capture.configure(1280, 720, "performance-matrix", false);
capture.set_camera([0.0, 8.0, 22.0], [0.0, 1.0, 0.0]);
capture.set_ui_enabled(false);

capture.set_render_mode("unlit");
capture.set_render_effects(false, false, false);
capture.wait(1.0);
capture.report("unlit_no_effects");

capture.set_render_mode("lit");
capture.set_render_effects(true, false, false);
capture.wait(1.0);
capture.report("shadows_only");

capture.set_render_effects(false, true, false);
capture.wait(1.0);
capture.report("gi_only");

capture.set_render_effects(true, true, false);
capture.wait(1.0);
capture.report("shadows_and_gi");
capture.exit();

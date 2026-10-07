// Low-angle receiver test for self-shadowing along object edges.
capture.configure(1280, 720, "shadow-shallow", false);
capture.set_render_effects(true, false, false);
capture.set_camera([2.8, 0.9, 3.8], [0.0, 0.45, 0.0]);
capture.screenshot("shallow_enabled");
capture.set_render_effects(false, false, false);
capture.screenshot("shallow_disabled");
capture.exit();

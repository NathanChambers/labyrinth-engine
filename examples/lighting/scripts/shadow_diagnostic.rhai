// Deterministic shadow comparison captures. Run with:
// cargo run -p lighting --release -- --windowed --capture-script=examples/lighting/scripts/shadow_diagnostic.rhai
capture.configure(1280, 720, "shadow-diagnostic", false);
capture.set_camera([5.0, 3.0, 6.0], [0.0, 1.0, 0.0]);
capture.set_render_effects(true, false, false);
capture.screenshot("shadows_enabled");
capture.set_render_effects(false, false, false);
capture.screenshot("shadows_disabled");
capture.set_render_effects(true, false, false);
capture.set_camera([1.8, 1.4, 3.2], [0.0, 0.45, 0.0]);
capture.screenshot("near_contact");
capture.set_camera([9.0, 5.0, 12.0], [0.0, 1.0, 0.0]);
capture.screenshot("far_cascade");
capture.exit();

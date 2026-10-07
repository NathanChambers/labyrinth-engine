capture.configure(1280, 720, "shadow-test-scene", false);
capture.set_render_effects(true, false, false);
capture.set_shadow_quality(2);
capture.wait(1.0);
capture.screenshot("fixed_overlap_cases");
capture.exit();

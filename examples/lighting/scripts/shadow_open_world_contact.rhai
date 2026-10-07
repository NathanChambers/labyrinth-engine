capture.configure(1280, 720, "shadow-open-world-contact", false);
capture.set_camera([0.0, 4.0, 2.5], [0.0, 0.0, 0.0]);
capture.set_render_effects(true, false, false);
capture.wait(1.0);
capture.set_shadow_quality(2);
capture.wait(0.25);
capture.screenshot("contact_enabled");
capture.exit();

capture.configure(1280, 720, "room-gi-only", false);
capture.set_render_mode("gi_only");
capture.set_render_effects(false, true, false);
capture.set_ambient_occlusion(false);
capture.set_camera([8.0, 12.0, 8.0], [0.0, 0.0, 0.0]);
capture.wait(1.5);
capture.screenshot("gi_only");
capture.exit();

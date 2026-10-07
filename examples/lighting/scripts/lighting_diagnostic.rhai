capture.configure(1280, 720, "lighting-diagnostic", false);
capture.set_camera([4.2, 2.8, 5.2], [0.0, 0.8, 0.0]);
capture.screenshot("overview");
capture.set_camera([1.8, 1.7, 3.2], [0.0, 0.8, 0.0]);
capture.screenshot("contact_shadows");
capture.set_camera([-1.8, 1.4, 1.8], [-0.2, 0.8, 0.0]);
capture.screenshot("color_bounce");
capture.exit();

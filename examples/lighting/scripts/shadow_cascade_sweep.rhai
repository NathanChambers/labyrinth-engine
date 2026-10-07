// Camera sweep used to inspect cascade transitions and stabilization.
capture.configure(1280, 720, "shadow-cascade-sweep", false);
capture.set_render_effects(true, false, false);
capture.set_camera([4.6, 2.8, 5.6], [0.0, 1.0, 0.0]);
capture.screenshot("step_00");
capture.set_camera([4.9, 2.8, 5.9], [0.0, 1.0, 0.0]);
capture.screenshot("step_01");
capture.set_camera([5.2, 2.8, 6.2], [0.0, 1.0, 0.0]);
capture.screenshot("step_02");
capture.set_camera([5.5, 2.8, 6.5], [0.0, 1.0, 0.0]);
capture.screenshot("step_03");
capture.exit();

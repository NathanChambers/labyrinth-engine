capture.configure(1280, 720, "lighting", false);
capture.set_camera([5.0, 3.0, 6.0], [0.0, 1.0, 0.0]);
capture.screenshot("initial");
capture.move_camera([7.0, 3.5, 3.0], [0.0, 1.0, 0.0], 1.5);
capture.wait(0.25);
capture.report("performance");
capture.sequence("orbit", 3, 0.25);
capture.exit();

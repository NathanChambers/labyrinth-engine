capture.configure(1280, 720, "large-lighting-diagnostic", false);
capture.set_camera([0.0, 2.8, 8.0], [0.0, 1.0, 0.0]);
capture.screenshot("courtyard");
capture.set_camera([5.5, 2.0, 2.0], [1.2, 0.8, 0.0]);
capture.screenshot("right_room");
capture.set_camera([-5.5, 2.0, 2.0], [-1.2, 0.8, 0.0]);
capture.screenshot("left_room");
capture.exit();

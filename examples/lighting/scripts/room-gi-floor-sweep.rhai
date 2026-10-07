capture.configure(1280, 720, "room-gi-floor-sweep", false);
capture.set_render_mode("gi_only");
capture.set_render_effects(false, true, false);
capture.set_ambient_occlusion(false);

capture.set_camera([0.0, 3.0, 9.0], [0.0, 0.0, 0.0]);
capture.wait(1.5);
capture.screenshot("floor_from_door");

capture.set_camera([0.0, 2.0, 2.5], [0.0, 0.5, -2.0]);
capture.wait(0.5);
capture.screenshot("floor_inside");

capture.set_camera([7.0, 2.0, 7.0], [0.0, 0.0, 0.0]);
capture.wait(0.5);
capture.screenshot("hallway_floor");
capture.exit();

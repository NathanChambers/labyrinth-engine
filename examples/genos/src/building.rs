use labyrinth::{Color, Vec3};

pub const SECTION_SIZE: f32 = 25.0;
const ROOM_HEIGHT: f32 = 3.0;
const ROOF_THICKNESS: f32 = 0.25;
const WALL_THICKNESS: f32 = 0.3;
const LAMP_HEIGHT: f32 = 2.6;
const YARD: f32 = 8.0;

#[derive(Clone, Copy)]
struct Opening {
    from: f32,
    to: f32,
    sill: f32,
    head: f32,
}

const fn window(from: f32, to: f32) -> Opening {
    Opening { from, to, sill: 0.9, head: 2.2 }
}

const fn high_window(from: f32, to: f32) -> Opening {
    Opening { from, to, sill: 1.7, head: 2.6 }
}

const fn wide_window(from: f32, to: f32) -> Opening {
    Opening { from, to, sill: 0.5, head: 2.6 }
}

const fn door(from: f32, to: f32) -> Opening {
    Opening { from, to, sill: 0.0, head: 2.3 }
}

const fn big_door(from: f32, to: f32) -> Opening {
    Opening { from, to, sill: 0.0, head: 2.7 }
}

const WEST_OUTSIDE: [Opening; 3] = [window(10.0, 14.0), window(18.0, 19.0), big_door(21.0, 24.0)];
const WEST_INSIDE: [Opening; 2] = [big_door(10.0, 14.0), big_door(19.0, 23.0)];
const NORTH_OUTSIDE: [Opening; 3] = [high_window(11.0, 12.0), window(13.0, 16.0), window(19.0, 22.0)];
const NORTH_INSIDE: [Opening; 2] = [big_door(10.0, 16.0), door(19.0, 22.0)];
const EAST_OUTSIDE: [Opening; 3] = [window(3.0, 5.0), big_door(10.0, 13.5), wide_window(17.5, 23.5)];
const SOUTH_OUTSIDE: [Opening; 3] = [window(3.0, 7.0), big_door(10.0, 14.0), wide_window(18.0, 24.0)];
const ROOF_HOLES: [[f32; 4]; 4] = [[0.6, 8.3, 7.4, 10.2], [10.5, 10.0, 15.5, 14.0], [17.5, 11.5, 24.5, 12.3], [20.0, 19.5, 23.0, 22.0]];
const PILLARS: [[f32; 2]; 4] = [[9.8, 9.3], [16.2, 9.3], [9.8, 14.7], [16.2, 14.7]];

#[derive(Clone, Copy)]
pub struct BoxSpec {
    pub center: Vec3,
    pub size: Vec3,
    pub color: Color,
    pub cylinder: bool,
}

#[derive(Clone, Copy)]
pub enum LampMotion {
    Orbit { radius: f32, rate: f32 },
    Sway { axis: Vec3, amplitude: f32, rate: f32 },
}

impl LampMotion {
    pub fn offset(self, time: f32) -> Vec3 {
        match self {
            Self::Orbit { radius, rate } => {
                let angle = rate * time;
                Vec3::new((angle.cos() - 1.0) * radius, 0.0, angle.sin() * radius)
            }
            Self::Sway { axis, amplitude, rate } => axis * (amplitude * (rate * time).sin()),
        }
    }
}

#[derive(Clone, Copy)]
pub struct LampSlot {
    pub home: Vec3,
    pub motion: LampMotion,
    pub tint: Color,
}

#[derive(Clone, Copy)]
enum BoxPath {
    Slide { from: Vec3, to: Vec3 },
    Orbit { center: Vec3, radius: f32 },
}

#[derive(Clone, Copy)]
pub struct MovingBox {
    pub size: Vec3,
    pub color: Color,
    path: BoxPath,
    period: f32,
    phase: f32,
    spin: f32,
}

impl MovingBox {
    pub fn pose(self, time: f32) -> (Vec3, f32) {
        let local_time = time + self.phase;
        let turn = std::f32::consts::TAU * local_time / self.period;
        let ground_position = match self.path {
            BoxPath::Slide { from, to } => from.lerp(to, 0.5 - 0.5 * turn.cos()),
            BoxPath::Orbit { center, radius } => center + Vec3::new(turn.cos(), 0.0, turn.sin()) * radius,
        };
        (ground_position + Vec3::Y * self.size.y * 0.5, self.spin * local_time)
    }
}

pub struct Building {
    pub cols: u32,
    pub rows: u32,
    pub static_boxes: Vec<BoxSpec>,
    pub moving_boxes: Vec<MovingBox>,
    pub lamps: Vec<LampSlot>,
}

impl Building {
    pub fn new(cols: u32, rows: u32, seed: u64) -> Self {
        let cols = cols.max(1);
        let rows = rows.max(1);
        let mut rng = Rng(seed);
        let mut building = Self { cols, rows, static_boxes: Vec::new(), moving_boxes: Vec::new(), lamps: Vec::new() };
        for row in 0..rows {
            for col in 0..cols {
                let origin = Vec3::new(col as f32 * SECTION_SIZE, 0.0, row as f32 * SECTION_SIZE);
                building.add_section(origin, col == 0, row == 0, col + 1 == cols, row + 1 == rows, &mut rng);
            }
        }
        building.lamps = lamp_slots();
        building
    }

    pub fn floor_center(&self) -> Vec3 {
        Vec3::new(self.cols as f32 * SECTION_SIZE * 0.5, 0.0, self.rows as f32 * SECTION_SIZE * 0.5)
    }

    pub fn floor_size(&self) -> (f32, f32) {
        (self.cols as f32 * SECTION_SIZE + YARD * 2.0, self.rows as f32 * SECTION_SIZE + YARD * 2.0)
    }

    fn add_section(&mut self, origin: Vec3, west_edge: bool, north_edge: bool, east_edge: bool, south_edge: bool, rng: &mut Rng) {
        let edge_west = if west_edge { &WEST_OUTSIDE[..] } else { &WEST_INSIDE[..] };
        let edge_north = if north_edge { &NORTH_OUTSIDE[..] } else { &NORTH_INSIDE[..] };
        self.wall_run(false, 0.0, 0.0, SECTION_SIZE, edge_west, origin, Color::WHITE);
        self.wall_run(true, 0.0, 0.0, SECTION_SIZE, edge_north, origin, Color::WHITE);
        if east_edge {
            self.wall_run(false, SECTION_SIZE, 0.0, SECTION_SIZE, &EAST_OUTSIDE, origin, Color::WHITE);
        }
        if south_edge {
            self.wall_run(true, SECTION_SIZE, 0.0, SECTION_SIZE, &SOUTH_OUTSIDE, origin, Color::WHITE);
        }

        self.add_inner_walls(origin);
        self.add_roof(origin);
        self.add_static_furniture(origin);
        self.add_moving_boxes(origin, rng);
    }

    fn add_inner_walls(&mut self, origin: Vec3) {
        self.wall_run(false, 8.0, 0.0, 8.0, &[], origin, Color::WHITE);
        self.wall_run(true, 8.0, 0.0, 8.0, &[Opening { from: 1.0, to: 7.0, sill: 0.0, head: 2.4 }], origin, Color::WHITE);
        self.wall_run(true, 10.5, 0.0, 9.0, &[], origin, Color::WHITE);
        self.wall_run(false, 17.0, 0.0, 8.0, &[door(2.0, 3.4)], origin, Color::WHITE);
        self.wall_run(true, 8.0, 17.0, SECTION_SIZE, &[door(20.0, 21.6)], origin, Color::WHITE);
        self.wall_run(true, 16.0, 17.0, SECTION_SIZE, &[door(19.0, 20.4)], origin, Color::WHITE);
        self.wall_run(false, 17.0, 16.0, SECTION_SIZE, &[big_door(19.0, 23.0)], origin, Color::WHITE);
        self.push_box(origin, 6.5, 0.8, 17.0, 7.0, 1.6, 0.2, Color::WHITE);
        self.push_box(origin, 0.21, 1.1, 4.0, 0.12, 2.2, 3.0, Color::rgb(0.15, 0.75, 0.2));
        self.push_box(origin, 21.5, 0.9, 0.45, 4.0, 1.8, 0.5, Color::rgb(0.85, 0.8, 0.7));
        self.push_box(origin, 3.5, 0.225, 12.5, 3.0, 0.45, 0.6, Color::rgb(0.55, 0.5, 0.45));
        self.push_box(origin, 5.5, 0.225, 22.0, 3.0, 0.45, 0.6, Color::rgb(0.55, 0.5, 0.45));
    }

    fn add_roof(&mut self, origin: Vec3) {
        let mut x_edges = vec![0.0, SECTION_SIZE];
        for hole in ROOF_HOLES {
            x_edges.extend([hole[0], hole[2]]);
        }
        x_edges.sort_by(f32::total_cmp);
        x_edges.dedup();
        for x_pair in x_edges.windows(2) {
            let (x0, x1) = (x_pair[0], x_pair[1]);
            let mut holes: Vec<[f32; 2]> = ROOF_HOLES.iter().filter(|hole| hole[0] <= x0 && hole[2] >= x1).map(|hole| [hole[1], hole[3]]).collect();
            holes.sort_by(|a, b| a[0].total_cmp(&b[0]));
            let mut z = 0.0;
            for [z0, z1] in holes {
                if z0 > z {
                    self.push_box(origin, (x0 + x1) * 0.5, ROOM_HEIGHT + ROOF_THICKNESS * 0.5, (z + z0) * 0.5, x1 - x0, ROOF_THICKNESS, z0 - z, Color::WHITE);
                }
                z = z.max(z1);
            }
            if z < SECTION_SIZE {
                self.push_box(origin, (x0 + x1) * 0.5, ROOM_HEIGHT + ROOF_THICKNESS * 0.5, (z + SECTION_SIZE) * 0.5, x1 - x0, ROOF_THICKNESS, SECTION_SIZE - z, Color::WHITE);
            }
        }
    }

    fn add_static_furniture(&mut self, origin: Vec3) {
        for [x, z] in PILLARS {
            self.push_box(origin, x, ROOM_HEIGHT * 0.5, z, 0.45, ROOM_HEIGHT, 0.45, Color::rgb(0.75, 0.75, 0.75));
        }
        self.push_box(origin, 2.0, 0.375, 2.5, 1.4, 0.75, 1.4, Color::rgb(0.9, 0.9, 0.9));
        self.push_box(origin, 5.5, 0.5, 4.0, 1.0, 1.0, 1.0, Color::rgb(0.85, 0.1, 0.1));
        self.static_boxes.push(BoxSpec { center: origin + Vec3::new(3.0, 0.6, 5.5), size: Vec3::new(0.8, 1.2, 0.8), color: Color::rgb(0.1, 0.25, 0.85), cylinder: true });
        self.push_box(origin, 23.3, 0.45, 6.3, 0.9, 0.9, 0.9, Color::rgb(0.9, 0.75, 0.1));
        self.static_boxes.push(BoxSpec { center: origin + Vec3::new(24.0, 0.3, 17.0), size: Vec3::new(1.2, 0.6, 1.2), color: Color::rgb(0.3, 0.6, 0.25), cylinder: true });
        self.push_box(origin, 19.5, 0.25, 23.5, 1.0, 0.5, 1.0, Color::rgb(0.6, 0.45, 0.3));
    }

    fn add_moving_boxes(&mut self, origin: Vec3, rng: &mut Rng) {
        let movers = [
            MovingBox {
                size: Vec3::splat(1.0),
                color: Color::rgb(0.9, 0.15, 0.1),
                path: BoxPath::Slide { from: origin + Vec3::new(10.5, 0.0, 19.5), to: origin + Vec3::new(15.5, 0.0, 19.5) },
                period: 8.0,
                phase: 0.0,
                spin: 0.6,
            },
            MovingBox {
                size: Vec3::splat(0.9),
                color: Color::rgb(0.15, 0.8, 0.2),
                path: BoxPath::Orbit { center: origin + Vec3::new(13.0, 0.0, 12.0), radius: 2.0 },
                period: 12.0,
                phase: 0.0,
                spin: -0.8,
            },
            MovingBox {
                size: Vec3::splat(0.8),
                color: Color::rgb(0.15, 0.3, 0.9),
                path: BoxPath::Slide { from: origin + Vec3::new(22.0, 0.0, 17.5), to: origin + Vec3::new(22.0, 0.0, 23.5) },
                period: 10.0,
                phase: 0.0,
                spin: 1.0,
            },
        ];
        self.moving_boxes.extend(movers.into_iter().map(|mut mover| {
            mover.phase = rng.range(mover.period);
            mover
        }));
    }

    fn wall_run(&mut self, along_x: bool, line: f32, from: f32, to: f32, openings: &[Opening], origin: Vec3, color: Color) {
        let mut cursor = from;
        for opening in openings {
            if opening.from > cursor {
                self.wall_piece(along_x, line, cursor, opening.from, 0.0, ROOM_HEIGHT, origin, color);
            }
            if opening.sill > 0.0 {
                self.wall_piece(along_x, line, opening.from, opening.to, 0.0, opening.sill, origin, color);
            }
            if opening.head < ROOM_HEIGHT {
                self.wall_piece(along_x, line, opening.from, opening.to, opening.head, ROOM_HEIGHT, origin, color);
            }
            cursor = opening.to;
        }
        if to > cursor {
            self.wall_piece(along_x, line, cursor, to, 0.0, ROOM_HEIGHT, origin, color);
        }
    }

    fn wall_piece(&mut self, along_x: bool, line: f32, start: f32, end: f32, base: f32, top: f32, origin: Vec3, color: Color) {
        let span = (end - start).abs();
        let center = if along_x { Vec3::new((start + end) * 0.5, (base + top) * 0.5, line) } else { Vec3::new(line, (base + top) * 0.5, (start + end) * 0.5) };
        let size = if along_x { Vec3::new(span, top - base, WALL_THICKNESS) } else { Vec3::new(WALL_THICKNESS, top - base, span) };
        self.static_boxes.push(BoxSpec { center: origin + center, size, color, cylinder: false });
    }

    fn push_box(&mut self, origin: Vec3, x: f32, y: f32, z: f32, width: f32, height: f32, depth: f32, color: Color) {
        self.static_boxes.push(BoxSpec { center: origin + Vec3::new(x, y, z), size: Vec3::new(width, height, depth), color, cylinder: false });
    }
}

pub fn distributed_lamps(building: &Building, count: usize) -> Vec<LampSlot> {
    let sections = (building.cols * building.rows) as usize;
    let count = count.min(sections * building.lamps.len());
    (0..count)
        .map(|index| {
            let section = index % sections;
            let nth = index / sections;
            let slot_index = (section + nth) % building.lamps.len();
            let origin = Vec3::new((section as u32 % building.cols) as f32 * SECTION_SIZE, 0.0, (section as u32 / building.cols) as f32 * SECTION_SIZE);
            let mut slot = building.lamps[slot_index];
            slot.home += origin;
            slot
        })
        .collect()
}

fn lamp_slots() -> Vec<LampSlot> {
    let x_axis = Vec3::X;
    let z_axis = Vec3::Z;
    let lamp = |u, v, motion, tint| LampSlot { home: Vec3::new(u, LAMP_HEIGHT, v), motion, tint };
    vec![
        lamp(4.5, 13.5, LampMotion::Sway { axis: z_axis, amplitude: 2.0, rate: 0.7 }, Color::rgb(1.0, 0.9, 0.75)),
        lamp(21.0, 4.0, LampMotion::Orbit { radius: 1.5, rate: 0.9 }, Color::rgb(0.8, 0.88, 1.0)),
        lamp(20.5, 21.0, LampMotion::Sway { axis: x_axis, amplitude: 1.5, rate: 0.6 }, Color::rgb(1.0, 0.9, 0.75)),
        lamp(12.5, 21.5, LampMotion::Orbit { radius: 1.5, rate: -0.8 }, Color::rgb(1.0, 0.9, 0.75)),
        lamp(12.5, 4.5, LampMotion::Sway { axis: x_axis, amplitude: 2.0, rate: 0.5 }, Color::rgb(0.8, 0.88, 1.0)),
        lamp(14.5, 6.0, LampMotion::Orbit { radius: 1.0, rate: 0.65 }, Color::rgb(0.8, 0.88, 1.0)),
        lamp(10.0, 21.5, LampMotion::Sway { axis: z_axis, amplitude: 1.2, rate: 0.85 }, Color::rgb(1.0, 0.9, 0.75)),
    ]
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = self.0;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    }

    fn range(&mut self, maximum: f32) -> f32 {
        (self.next() >> 40) as f32 / (1_u32 << 24) as f32 * maximum
    }
}

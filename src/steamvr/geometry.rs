use super::abi::Matrix;
use std::time::Instant;

const DISTANCE: f32 = 1.05;
const DROP: f32 = 0.12;
const MIN_CUTOFF_HZ: f32 = 6.0;
const DERIVATIVE_CUTOFF_HZ: f32 = 10.0;
const SPEED_COEFFICIENT: f32 = 16.0;

#[derive(Default)]
pub struct PointerFilter(Option<PointerSample>);

struct PointerSample {
    raw: [f32; 2],
    filtered: [f32; 2],
    velocity: [f32; 2],
    time: Instant,
}

impl PointerFilter {
    pub fn update(&mut self, uv: [f32; 2], now: Instant, aspect: f32) -> [f32; 2] {
        let Some(previous) = &self.0 else {
            self.0 = Some(PointerSample {
                raw: uv,
                filtered: uv,
                velocity: [0.0; 2],
                time: now,
            });
            return uv;
        };
        let elapsed = now.duration_since(previous.time).as_secs_f32();
        if elapsed == 0.0 {
            return previous.filtered;
        }

        // One Euro filter with a shared cutoff based on speed in panel widths per second.
        let derivative_blend = smoothing_factor(DERIVATIVE_CUTOFF_HZ, elapsed);
        let velocity: [f32; 2] = std::array::from_fn(|i| {
            let derivative = (uv[i] - previous.raw[i]) / elapsed;
            previous.velocity[i] + derivative_blend * (derivative - previous.velocity[i])
        });
        let speed = velocity[0].hypot(velocity[1] / aspect);
        let blend = smoothing_factor(MIN_CUTOFF_HZ + SPEED_COEFFICIENT * speed, elapsed);
        let filtered =
            std::array::from_fn(|i| previous.filtered[i] + blend * (uv[i] - previous.filtered[i]));
        self.0 = Some(PointerSample {
            raw: uv,
            filtered,
            velocity,
            time: now,
        });
        filtered
    }

    pub fn reset(&mut self) {
        self.0 = None;
    }
}

fn smoothing_factor(cutoff_hz: f32, elapsed: f32) -> f32 {
    let rate = std::f32::consts::TAU * cutoff_hz * elapsed;
    rate / (1.0 + rate)
}

pub fn anchor(head: &Matrix) -> Matrix {
    // Apply the offset in headset space so pitch, yaw and roll are all respected.
    Matrix(head.0.map(|[right, up, back, position]| {
        [right, up, back, position - up * DROP - back * DISTANCE]
    }))
}

/// Express a standing-space transform in the controller's local coordinates.
pub fn relative_transform(controller: &Matrix, panel: &Matrix) -> Matrix {
    let delta = std::array::from_fn(|i| panel.0[i][3] - controller.0[i][3]);
    Matrix(std::array::from_fn(|axis| {
        let basis = controller.0.map(|row| row[axis]);
        let [x, y, z] = std::array::from_fn(|column| dot(basis, panel.0.map(|row| row[column])));
        [x, y, z, dot(basis, delta)]
    }))
}

/// Bring a controller-local transform back into standing space.
pub fn compose(controller: &Matrix, local: &Matrix) -> Matrix {
    Matrix(controller.0.map(|[x, y, z, position]| {
        let mut row =
            std::array::from_fn(|i| x * local.0[0][i] + y * local.0[1][i] + z * local.0[2][i]);
        row[3] += position;
        row
    }))
}

pub fn hit(panel: &Matrix, aim: &Matrix, width: f32, aspect: f32) -> Option<[f32; 2]> {
    let column = |matrix: &Matrix, index| matrix.0.map(|row| row[index]);
    let forward = column(aim, 2).map(|value| -value);
    let normal = column(panel, 2);
    let origin = column(aim, 3);
    let center = column(panel, 3);
    let projection = dot(forward, normal);
    if projection > -0.001 {
        return None;
    }
    let delta = std::array::from_fn(|i| center[i] - origin[i]);
    let distance = dot(delta, normal) / projection;
    if distance <= 0.0 {
        return None;
    }
    let offset = std::array::from_fn(|i| forward[i] * distance - delta[i]);
    let u = 0.5 + dot(offset, column(panel, 0)) / width;
    let v = 0.5 - dot(offset, column(panel, 1)) / (width / aspect);
    ((0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v)).then_some([u, v])
}

pub fn cursor_transform(panel: &Matrix, [u, v]: [f32; 2], width: f32, aspect: f32) -> Matrix {
    let x = (u - 0.5) * width;
    let y = (0.5 - v) * (width / aspect);
    // Place the cursor just in front of the panel in the same standing space.
    Matrix(panel.0.map(|[right, up, back, position]| {
        [
            right,
            up,
            back,
            position + right * x + up * y + back * 0.001,
        ]
    }))
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn pointer_filter_damps_jitter_and_tracks_fast_motion() {
        let start = Instant::now();
        let next = start + Duration::from_millis(11);
        let center = [0.5, 0.5];
        let mut slow = PointerFilter::default();
        let mut fast = PointerFilter::default();
        assert_eq!(slow.update(center, start, 2.0), center);
        assert_eq!(fast.update(center, start, 2.0), center);

        let jitter = 0.501;
        let movement = 0.7;
        let damped = slow.update([jitter, 0.5], next, 2.0)[0];
        let tracked = fast.update([movement, 0.5], next, 2.0)[0];
        assert!(damped > 0.5 && damped < 0.5 + (jitter - 0.5) * 0.5);
        assert!(tracked > 0.5 + (movement - 0.5) * 0.8 && tracked <= movement);
        assert_eq!(fast.update(center, next, 2.0)[0], tracked);

        fast.reset();
        assert_eq!(fast.update(center, next, 2.0), center);
    }

    #[test]
    fn intersects_the_keyboard_front_and_flips_vertical_coordinates() {
        let head = Matrix([[1., 0., 0., 0.], [0., 1., 0., 1.6], [0., 0., 1., 0.]]);
        let panel = anchor(&head);
        let mut aim = head;
        aim.0[1][3] -= DROP;
        assert_eq!(hit(&panel, &aim, 1., 2.), Some([0.5, 0.5]));
        aim.0[1][3] += 0.125;
        assert_eq!(hit(&panel, &aim, 1., 2.), Some([0.5, 0.25]));
        aim.0[0][3] += 1.;
        assert_eq!(hit(&panel, &aim, 1., 2.), None);
    }

    #[test]
    fn cursor_tracks_keyboard_coordinates_and_width() {
        let panel = Matrix([[1., 0., 0., 0.], [0., 1., 0., 0.], [0., 0., 1., 0.]]);
        let aim = Matrix([[1., 0., 0., 0.5], [0., 1., 0., 0.25], [0., 0., 1., 1.]]);
        for width in [2., 4.] {
            let uv = hit(&panel, &aim, width, 2.).unwrap();
            let cursor = cursor_transform(&panel, uv, width, 2.);
            assert_eq!(cursor.0[0][3], aim.0[0][3]);
            assert_eq!(cursor.0[1][3], aim.0[1][3]);
            assert!(cursor.0[2][3] > 0.);
        }
    }

    #[test]
    fn cursor_follows_a_translated_and_rotated_panel() {
        let panel = Matrix([[0., 0., 1., 2.], [1., 0., 0., 3.], [0., 1., 0., 4.]]);
        let cursor = cursor_transform(&panel, [0.75, 0.25], 2., 2.);
        assert_eq!(
            cursor.0,
            [[0., 0., 1., 2.001], [1., 0., 0., 3.5], [0., 1., 0., 4.25]]
        );
    }

    #[test]
    fn grabbed_panel_keeps_the_point_under_the_controller_ray() {
        let start = Matrix([[0., 0., 1., 2.], [0., 1., 0., 1.], [-1., 0., 0., 3.]]);
        let panel = Matrix([[0., 0., 1., 1.], [0., 1., 0., 0.75], [-1., 0., 0., 3.5]]);
        let offset = relative_transform(&start, &panel);
        let uv = hit(&panel, &start, 2., 2.).unwrap();
        assert_ne!(uv, [0.5, 0.5]);
        assert_eq!(compose(&start, &offset).0, panel.0);

        let moved = Matrix([[0., -1., 0., 4.], [1., 0., 0., 2.], [0., 0., 1., 5.]]);
        let grabbed = compose(&moved, &offset);
        assert_eq!(hit(&grabbed, &moved, 2., 2.), Some(uv));
    }
}

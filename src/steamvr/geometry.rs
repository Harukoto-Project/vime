use super::abi::Matrix;
use std::time::Instant;

const DISTANCE: f32 = 1.05;
const DROP: f32 = 0.12;

#[derive(Default)]
pub struct PointerFilter(Option<([f32; 2], Instant)>);

impl PointerFilter {
    pub fn update(&mut self, uv: [f32; 2], now: Instant, aspect: f32) -> [f32; 2] {
        let filtered = if let Some((previous, time)) = self.0 {
            let elapsed = now.duration_since(time).as_secs_f32();
            let dx = uv[0] - previous[0];
            let dy = uv[1] - previous[1];
            // Small movements are damped. Larger movements follow the controller faster.
            let travel = dx.hypot(dy / aspect);
            let response = 0.024 - 0.018 * (travel / 0.04).min(1.);
            let blend = 1. - (-elapsed / response).exp();
            [previous[0] + dx * blend, previous[1] + dy * blend]
        } else {
            uv
        };
        self.0 = Some((filtered, now));
        filtered
    }

    pub fn reset(&mut self) {
        self.0 = None;
    }
}

pub fn anchor(head: &Matrix) -> Matrix {
    // Apply the offset in headset space so pitch, yaw and roll are all respected.
    Matrix(head.0.map(|[right, up, back, position]| {
        [right, up, back, position - up * DROP - back * DISTANCE]
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
}
